// Narrow AVFoundation video-only adapter. No AppKit rendering or canvas/GPU access.
#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#include <atomic>
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
extern "C" void rastera_capture_pixels(void *,uint32_t,uint32_t,size_t,const uint8_t *,size_t);
extern "C" void rastera_capture_state(void *,const char *);
extern "C" void rastera_capture_negotiated(void *,uint32_t,uint32_t,double);
extern "C" void rastera_capture_release(void *);
static std::atomic<uint64_t> nextCaptureSession{1};
static dispatch_queue_t SessionQueue(){static dispatch_queue_t q=dispatch_queue_create("org.rastera.capture.sessions",DISPATCH_QUEUE_SERIAL);return q;}
static dispatch_group_t SessionGroup(){static dispatch_group_t g=dispatch_group_create();return g;}
static BOOL SDRFormat(AVCaptureDeviceFormat *format){
    FourCharCode c=CMFormatDescriptionGetMediaSubType(format.formatDescription);
    return c==kCVPixelFormatType_32BGRA||c==kCVPixelFormatType_32ARGB||c==kCVPixelFormatType_422YpCbCr8||c==kCVPixelFormatType_422YpCbCr8_yuvs||c==kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange||c==kCVPixelFormatType_420YpCbCr8BiPlanarFullRange||c==kCMVideoCodecType_JPEG;
}
static NSArray<AVCaptureDevice *> *Devices(){return [AVCaptureDeviceDiscoverySession discoverySessionWithDeviceTypes:@[AVCaptureDeviceTypeBuiltInWideAngleCamera,AVCaptureDeviceTypeExternal,AVCaptureDeviceTypeContinuityCamera] mediaType:AVMediaTypeVideo position:AVCaptureDevicePositionUnspecified].devices;}
extern "C" bool rastera_camera_bundle_valid(){return [NSBundle.mainBundle.bundleIdentifier isEqual:@"org.rastera.desktop.dev"]&&[NSBundle.mainBundle objectForInfoDictionaryKey:@"NSCameraUsageDescription"]!=nil;}
extern "C" int rastera_camera_authorization(){return int([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]);}
extern "C" bool rastera_camera_request(){
    if(![NSBundle.mainBundle objectForInfoDictionaryKey:@"NSCameraUsageDescription"])return false;
    if([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]==AVAuthorizationStatusNotDetermined)[AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted){std::fprintf(stderr,"capture_permission granted=%d\n",int(granted));}];
    return true;
}
extern "C" char *rastera_capture_devices(){
    NSMutableArray *rows=[NSMutableArray array];
    for(AVCaptureDevice *device in Devices()){
        NSMutableArray *modes=[NSMutableArray array];NSMutableSet *seen=[NSMutableSet set];
        for(AVCaptureDeviceFormat *format in device.formats){if(!SDRFormat(format))continue;CMVideoDimensions d=CMVideoFormatDescriptionGetDimensions(format.formatDescription);if(d.width<=0||d.height<=0||d.width>1920||d.height>1920||int64_t(d.width)*d.height>1920*1080)continue;
            for(AVFrameRateRange *range in format.videoSupportedFrameRateRanges){for(NSNumber *rate in @[@1.,@5.,@10.,@15.,@23.976,@24.,@25.,@29.97,@30.,@50.,@59.94,@60.,@(range.minFrameRate),@(std::min(60.,range.maxFrameRate))]){double fps=rate.doubleValue;if(fps<1.||fps>60.||fps<range.minFrameRate-0.001||fps>range.maxFrameRate+0.001)continue;NSString *key=[NSString stringWithFormat:@"%dx%d@%.3f",d.width,d.height,fps];if([seen containsObject:key])continue;[seen addObject:key];[modes addObject:@{@"width":@(d.width),@"height":@(d.height),@"fps":rate}];}}
        }
        [rows addObject:@{@"id":device.uniqueID,@"name":device.localizedName,@"model":device.modelID?:@"",@"modes":modes}];
    }
    NSData *json=[NSJSONSerialization dataWithJSONObject:rows options:0 error:nil];return strdup([[NSString alloc]initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
}
@interface RasteraCapture : NSObject<AVCaptureVideoDataOutputSampleBufferDelegate>{
    AVCaptureSession *_session;AVCaptureVideoDataOutput *_output;dispatch_queue_t _delivery;
    uint64_t _sessionID,_frames;
    void *_context;std::atomic<bool> _stopped;NSMutableArray *_observers;BOOL _configuring;AVCaptureDevice *_lockedDevice;
}
- (void)start:(void *)context device:(NSString *)identity width:(uint32_t)width height:(uint32_t)height fps:(double)fps;
- (void)shutdown;
@end
@implementation RasteraCapture
- (void)start:(void *)context device:(NSString *)identity width:(uint32_t)width height:(uint32_t)height fps:(double)fps {
    _sessionID=nextCaptureSession.fetch_add(1);_context=context;_delivery=dispatch_queue_create("org.rastera.capture.delivery",DISPATCH_QUEUE_SERIAL);_observers=[NSMutableArray array];dispatch_group_enter(SessionGroup());
    dispatch_async(SessionQueue(),^{@autoreleasepool{@try{
        if(self->_stopped)return;
        if([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]!=AVAuthorizationStatusAuthorized){rastera_capture_state(self->_context,"Permission Required");return;}
        AVCaptureDevice *device=nil;for(AVCaptureDevice *candidate in Devices())if([candidate.uniqueID isEqual:identity]){device=candidate;break;}
        if(!device){rastera_capture_state(self->_context,"Disconnected");return;}
        AVCaptureDeviceFormat *chosen=nil;AVFrameRateRange *chosenRange=nil;
        for(AVCaptureDeviceFormat *format in device.formats){CMVideoDimensions d=CMVideoFormatDescriptionGetDimensions(format.formatDescription);if(!SDRFormat(format)||d.width!=int32_t(width)||d.height!=int32_t(height))continue;for(AVFrameRateRange *range in format.videoSupportedFrameRateRanges)if(fps>=range.minFrameRate-0.001&&fps<=range.maxFrameRate+0.001){chosen=format;chosenRange=range;break;}if(chosen)break;}
        if(!chosen){rastera_capture_state(self->_context,"Error: requested SDR mode is unavailable; reassign explicitly");return;}
        NSError *error=nil;AVCaptureDeviceInput *input=[AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
        if(!input){rastera_capture_state(self->_context,[[NSString stringWithFormat:@"Error: %@",error.localizedDescription] UTF8String]);return;}
        self->_session=[AVCaptureSession new];[self->_session beginConfiguration];self->_configuring=YES;
        AVCaptureSessionPreset preset=width==1280&&height==720?AVCaptureSessionPreset1280x720:(width==1920&&height==1080?AVCaptureSessionPreset1920x1080:(width==640&&height==480?AVCaptureSessionPreset640x480:AVCaptureSessionPresetHigh));
        if([self->_session canSetSessionPreset:preset])self->_session.sessionPreset=preset;
        self->_output=[AVCaptureVideoDataOutput new];self->_output.alwaysDiscardsLateVideoFrames=YES;self->_output.videoSettings=@{(id)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA),(id)kCVPixelBufferWidthKey:@(width),(id)kCVPixelBufferHeightKey:@(height)};
        if(![self->_session canAddInput:input]||![self->_session canAddOutput:self->_output]){[self->_session commitConfiguration];self->_configuring=NO;rastera_capture_state(self->_context,"Error: device cannot provide BGRA video output");return;}
        [self->_session addInput:input];[self->_session addOutput:self->_output];
        if(![device lockForConfiguration:&error]){[self->_session commitConfiguration];self->_configuring=NO;rastera_capture_state(self->_context,"Error: device configuration is locked by another application");return;}
        self->_lockedDevice=device;device.activeFormat=chosen;CMTime interval=CMTimeMakeWithSeconds(1./fps,1000000000);if(CMTimeCompare(interval,chosenRange.minFrameDuration)<0)interval=chosenRange.minFrameDuration;if(CMTimeCompare(interval,chosenRange.maxFrameDuration)>0)interval=chosenRange.maxFrameDuration;device.activeVideoMinFrameDuration=interval;device.activeVideoMaxFrameDuration=interval;[device unlockForConfiguration];self->_lockedDevice=nil;
        AVCaptureConnection *connection=[self->_output connectionWithMediaType:AVMediaTypeVideo];if([connection isVideoRotationAngleSupported:0])connection.videoRotationAngle=0;if(connection.isVideoMirroringSupported){connection.automaticallyAdjustsVideoMirroring=NO;connection.videoMirrored=NO;}
        [self->_output setSampleBufferDelegate:self queue:self->_delivery];[self->_session commitConfiguration];self->_configuring=NO;
        __weak RasteraCapture *weak=self;
        for(NSString *name in @[AVCaptureSessionRuntimeErrorNotification,AVCaptureSessionWasInterruptedNotification,AVCaptureDeviceWasDisconnectedNotification]){
            id token=[NSNotificationCenter.defaultCenter addObserverForName:name object:[name isEqual:AVCaptureDeviceWasDisconnectedNotification]?device:self->_session queue:nil usingBlock:^(NSNotification *note){RasteraCapture *owner=weak;if(!owner)return;dispatch_async(owner->_delivery,^{if(owner->_stopped)return;NSString *message=[note.name isEqual:AVCaptureDeviceWasDisconnectedNotification]?@"Disconnected":([note.name isEqual:AVCaptureSessionWasInterruptedNotification]?@"No Signal":@"Error: AVFoundation runtime failure; retry source");rastera_capture_state(owner->_context,message.UTF8String);});}];[self->_observers addObject:token];
        }
        [self->_session startRunning];
        if(!self->_session.running){rastera_capture_state(self->_context,"Error: capture session did not start");return;}
        double actual=1./CMTimeGetSeconds(device.activeVideoMinFrameDuration);rastera_capture_negotiated(self->_context,width,height,actual);
        CMVideoDimensions sensor=CMVideoFormatDescriptionGetDimensions(device.activeFormat.formatDescription);
        std::fprintf(stderr,"capture_negotiated device=%s width=%u height=%u input_width=%d input_height=%d fps=%.6f pixel_format=BGRA rotation=0 mirrored=0\n",device.localizedName.UTF8String,width,height,sensor.width,sensor.height,actual);
    }@catch(NSException *exception){if(self->_lockedDevice){[self->_lockedDevice unlockForConfiguration];self->_lockedDevice=nil;}if(self->_configuring){@try{[self->_session commitConfiguration];}@catch(NSException *ignored){}self->_configuring=NO;}rastera_capture_state(self->_context,[[NSString stringWithFormat:@"Error: AVFoundation negotiation: %@",exception.reason] UTF8String]);}}});
}
- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)sample fromConnection:(AVCaptureConnection *)connection {
    if(_stopped)return;CVPixelBufferRef image=CMSampleBufferGetImageBuffer(sample);if(!image)return;
    if(CVPixelBufferGetPixelFormatType(image)!=kCVPixelFormatType_32BGRA||CVPixelBufferIsPlanar(image)){rastera_capture_state(_context,"Error: negotiated frame is not SDR BGRA");return;}
    CFTypeRef transfer=CVBufferCopyAttachment(image,kCVImageBufferTransferFunctionKey,nullptr);
    bool hdr=transfer&&(CFEqual(transfer,kCVImageBufferTransferFunction_ITU_R_2100_HLG)||CFEqual(transfer,kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ));if(transfer)CFRelease(transfer);
    if(hdr){rastera_capture_state(_context,"Error: HDR transfer is unsupported; select an SDR mode");return;}
    NSDictionary *aspect=CFBridgingRelease(CVBufferCopyAttachment(image,kCVImageBufferPixelAspectRatioKey,nullptr));
    if(aspect&&[aspect[(id)kCVImageBufferPixelAspectRatioHorizontalSpacingKey] doubleValue]!=[aspect[(id)kCVImageBufferPixelAspectRatioVerticalSpacingKey] doubleValue]){rastera_capture_state(_context,"Error: non-square capture pixels are unsupported");return;}
    if(CVPixelBufferLockBaseAddress(image,kCVPixelBufferLock_ReadOnly)!=kCVReturnSuccess){rastera_capture_state(_context,"Error: camera buffer lock failed");return;}
    const uint8_t *base=(const uint8_t *)CVPixelBufferGetBaseAddress(image);if(base)rastera_capture_pixels(_context,uint32_t(CVPixelBufferGetWidth(image)),uint32_t(CVPixelBufferGetHeight(image)),CVPixelBufferGetBytesPerRow(image),base,CVPixelBufferGetDataSize(image));
    CVPixelBufferUnlockBaseAddress(image,kCVPixelBufferLock_ReadOnly);
    if(++_frames%30==0)std::fprintf(stderr,"capture_delivery session=%llu frames=%llu width=%zu height=%zu callback_host_s=%.9f\n",(unsigned long long)_sessionID,(unsigned long long)_frames,CVPixelBufferGetWidth(image),CVPixelBufferGetHeight(image),NSProcessInfo.processInfo.systemUptime);
}
- (void)shutdown {
    if(_stopped.exchange(true))return;
    dispatch_async(SessionQueue(),^{@autoreleasepool{
        for(id token in self->_observers)[NSNotificationCenter.defaultCenter removeObserver:token];[self->_observers removeAllObjects];
        @try{[self->_session stopRunning];}@catch(NSException *exception){std::fprintf(stderr,"capture_stop_error %s\n",exception.reason.UTF8String);}
        [self->_output setSampleBufferDelegate:nil queue:nullptr];
        dispatch_sync(self->_delivery,^{rastera_capture_release(self->_context);self->_context=nullptr;});self->_session=nil;self->_output=nil;dispatch_group_leave(SessionGroup());
    }});
}
@end
extern "C" void *rastera_capture_create(void *context,const char *device,uint32_t width,uint32_t height,double fps){RasteraCapture *capture=[RasteraCapture new];[capture start:context device:@(device) width:width height:height fps:fps];return (__bridge_retained void *)capture;}
extern "C" void rastera_capture_destroy(void *handle){RasteraCapture *capture=(__bridge_transfer RasteraCapture *)handle;[capture shutdown];}
extern "C" bool rastera_capture_drain(){bool drained=dispatch_group_wait(SessionGroup(),dispatch_time(DISPATCH_TIME_NOW,5*NSEC_PER_SEC))==0;std::fprintf(stderr,"capture_shutdown drained=%d\n",int(drained));return drained;}
