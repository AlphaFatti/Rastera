#include <cmath>
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <QuartzCore/QuartzCore.h>
extern "C" void rastera_media_pixels(void*,uint32_t,uint32_t,size_t,const uint8_t*,size_t);
static NSDictionary *probe(NSString *path){
 AVURLAsset *asset=[AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:path] options:@{AVURLAssetPreferPreciseDurationAndTimingKey:@YES}];
 AVAssetTrack *track=[[asset tracksWithMediaType:AVMediaTypeVideo] firstObject];
 if(!track||!asset.playable||asset.hasProtectedContent)return @{@"error":@"AVFoundation cannot play this local file (missing/unsupported video)"};
 double duration=CMTimeGetSeconds(asset.duration);if(!std::isfinite(duration)||duration<=0)return @{@"error":@"Video requires a finite positive duration"};
 CGRect rect=CGRectApplyAffineTransform(CGRectMake(0,0,track.naturalSize.width,track.naturalSize.height),track.preferredTransform);
 for(id desc in track.formatDescriptions){NSDictionary *ext=(__bridge NSDictionary*)CMFormatDescriptionGetExtensions((__bridge CMFormatDescriptionRef)desc);NSString *tf=ext[(__bridge NSString*)kCVImageBufferTransferFunctionKey];if([tf containsString:@"2084"]||[tf containsString:@"HLG"])return @{@"error":@"HDR video is not supported in this SDR milestone"};}
 return @{@"size":@[@(llround(fabs(rect.size.width))),@(llround(fabs(rect.size.height)))],@"duration":@(duration),@"gif":@NO,@"path":path};
}
extern "C" char *rastera_video_probe(const char *path){@autoreleasepool{NSDictionary *data=probe(@(path));NSData *json=[NSJSONSerialization dataWithJSONObject:data options:0 error:nil];return strdup([[NSString alloc]initWithData:json encoding:NSUTF8StringEncoding].UTF8String);}}
@interface RasteraFileVideo:NSObject
@property AVPlayer *player;
@property AVPlayerItemVideoOutput *output;
@end
@implementation RasteraFileVideo @end
extern "C" void *rastera_video_create(const char *path){@autoreleasepool{
 AVURLAsset *asset=[AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:@(path)] options:nil];AVPlayerItem *item=[AVPlayerItem playerItemWithAsset:asset];
 item.videoComposition=[AVVideoComposition videoCompositionWithPropertiesOfAsset:asset];
 RasteraFileVideo *v=[RasteraFileVideo new];v.output=[[AVPlayerItemVideoOutput alloc]initWithPixelBufferAttributes:@{(__bridge NSString*)kCVPixelBufferPixelFormatTypeKey:@(kCVPixelFormatType_32BGRA)}];v.output.suppressesPlayerRendering=YES;
 [item addOutput:v.output];v.player=[AVPlayer playerWithPlayerItem:item];v.player.muted=YES;v.player.volume=0.;v.player.actionAtItemEnd=AVPlayerActionAtItemEndPause;
 return (__bridge_retained void*)v;
}}
extern "C" int rastera_video_step(void *handle,void *context,bool playing,double seek,double *position){@autoreleasepool{
 RasteraFileVideo *v=(__bridge RasteraFileVideo*)handle;
 if(v.player.currentItem.status==AVPlayerItemStatusFailed)return -1;
 if(seek>=0.)[v.player seekToTime:CMTimeMakeWithSeconds(seek,60000) toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
 if(playing){if(v.player.rate==0.)[v.player play];}else [v.player pause];
 CMTime t=[v.output itemTimeForHostTime:CACurrentMediaTime()];*position=CMTimeGetSeconds(v.player.currentTime);
 if([v.output hasNewPixelBufferForItemTime:t]){CVPixelBufferRef b=[v.output copyPixelBufferForItemTime:t itemTimeForDisplay:nil];if(b){CVPixelBufferLockBaseAddress(b,kCVPixelBufferLock_ReadOnly);rastera_media_pixels(context,(uint32_t)CVPixelBufferGetWidth(b),(uint32_t)CVPixelBufferGetHeight(b),CVPixelBufferGetBytesPerRow(b),(const uint8_t*)CVPixelBufferGetBaseAddress(b),CVPixelBufferGetDataSize(b));CVPixelBufferUnlockBaseAddress(b,kCVPixelBufferLock_ReadOnly);CVPixelBufferRelease(b);}}
 return v.player.currentItem.status==AVPlayerItemStatusReadyToPlay?1:0;
}}
extern "C" void rastera_video_destroy(void *handle){@autoreleasepool{RasteraFileVideo *v=(__bridge_transfer RasteraFileVideo*)handle;[v.player pause];[v.player.currentItem removeOutput:v.output];[v.player replaceCurrentItemWithPlayerItem:nil];}}
