// Narrow macOS adapters. Only bounded, preallocated PCM/ring work runs in the IOProc.
#import <AVFoundation/AVFoundation.h>
#import <Cocoa/Cocoa.h>
#import <CoreAudio/CoreAudio.h>
#import <AudioToolbox/AudioToolbox.h>
#import <QuartzCore/QuartzCore.h>
#import <mach/mach_time.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstring>
#include <cstddef>
#include <memory>

static AudioObjectPropertyAddress address(AudioObjectPropertySelector selector,
        AudioObjectPropertyScope scope=kAudioObjectPropertyScopeGlobal,
        UInt32 element=kAudioObjectPropertyElementMain) { return {selector,scope,element}; }
template<class T> static bool property(AudioObjectID id,AudioObjectPropertySelector key,T &value,
        AudioObjectPropertyScope scope=kAudioObjectPropertyScopeGlobal) {
    auto a=address(key,scope); UInt32 bytes=sizeof(value);
    return AudioObjectGetPropertyData(id,&a,0,nullptr,&bytes,&value)==noErr;
}
static NSString *stringProperty(AudioObjectID id,AudioObjectPropertySelector key,
        AudioObjectPropertyScope scope=kAudioObjectPropertyScopeGlobal,UInt32 element=0) {
    CFStringRef value=nullptr; auto a=address(key,scope,element); UInt32 bytes=sizeof(value);
    if(AudioObjectGetPropertyData(id,&a,0,nullptr,&bytes,&value)!=noErr||!value) return @"";
    return CFBridgingRelease(value);
}
static NSArray<NSNumber*> *deviceIDs() {
    auto a=address(kAudioHardwarePropertyDevices); UInt32 bytes=0;
    if(AudioObjectGetPropertyDataSize(kAudioObjectSystemObject,&a,0,nullptr,&bytes)!=noErr) return @[];
    std::unique_ptr<AudioDeviceID[]> ids(new AudioDeviceID[bytes/sizeof(AudioDeviceID)]);
    if(AudioObjectGetPropertyData(kAudioObjectSystemObject,&a,0,nullptr,&bytes,ids.get())!=noErr) return @[];
    NSMutableArray *result=[NSMutableArray new];
    for(UInt32 i=0;i<bytes/sizeof(AudioDeviceID);i++) [result addObject:@(ids[i])];
    return result;
}
static UInt32 channels(AudioDeviceID id,AudioObjectPropertyScope scope) {
    auto a=address(kAudioDevicePropertyStreamConfiguration,scope); UInt32 bytes=0;
    if(AudioObjectGetPropertyDataSize(id,&a,0,nullptr,&bytes)!=noErr) return 0;
    std::unique_ptr<uint8_t[]> memory(new uint8_t[bytes]); auto *list=(AudioBufferList*)memory.get();
    if(AudioObjectGetPropertyData(id,&a,0,nullptr,&bytes,list)!=noErr) return 0;
    UInt32 result=0; for(UInt32 i=0;i<list->mNumberBuffers;i++) result+=list->mBuffers[i].mNumberChannels;
    return result;
}
static NSArray *channelNames(AudioDeviceID id,UInt32 count,AudioObjectPropertyScope scope) {
    NSMutableArray *names=[NSMutableArray new];
    for(UInt32 c=0;c<count;c++) {
        NSString *name=stringProperty(id,kAudioObjectPropertyElementName,scope,c+1);
        [names addObject:name.length?name:[NSString stringWithFormat:@"Channel %u",c+1]];
    }
    return names;
}
static char *json(id value) {
    NSData *data=[NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    if(!data) return nullptr;
    return strdup([[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}
extern "C" char *rastera_audio_devices() { @autoreleasepool {
    NSMutableArray *rows=[NSMutableArray new];
    for(NSNumber *number in deviceIDs()) {
        AudioDeviceID id=number.unsignedIntValue; Float64 rate=0; UInt32 frames=0,latency=0,safety=0;
        property(id,kAudioDevicePropertyNominalSampleRate,rate);
        property(id,kAudioDevicePropertyBufferFrameSize,frames);
        property(id,kAudioDevicePropertyLatency,latency,kAudioDevicePropertyScopeOutput);
        property(id,kAudioDevicePropertySafetyOffset,safety,kAudioDevicePropertyScopeOutput);
        UInt32 in=channels(id,kAudioDevicePropertyScopeInput),out=channels(id,kAudioDevicePropertyScopeOutput);
        [rows addObject:@{@"uid":stringProperty(id,kAudioDevicePropertyDeviceUID),
            @"name":stringProperty(id,kAudioObjectPropertyName),@"rate":@(rate),@"buffer_frames":@(frames),
            @"output_channels":@(out),@"input_channels":@(in),
            @"output_names":channelNames(id,out,kAudioDevicePropertyScopeOutput),
            @"input_names":channelNames(id,in,kAudioDevicePropertyScopeInput),@"latency_frames":@(latency+safety)}];
    }
    return json(rows);
}}
extern "C" int rastera_audio_permission(bool request) { @autoreleasepool {
    AVAuthorizationStatus status=[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if(request&&status==AVAuthorizationStatusNotDetermined)
        [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted){(void)granted;}];
    return (int)status;
}}
static id finiteTime(CMTime time) {double value=CMTimeGetSeconds(time);return std::isfinite(value)?(id)@(value):(id)NSNull.null;}
extern "C" char *rastera_audio_probe(const char *path) { @autoreleasepool {
    AVURLAsset *asset=[AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:@(path)] options:nil];
    NSMutableArray *rows=[NSMutableArray new];
    for(AVAssetTrack *track in [asset tracksWithMediaType:AVMediaTypeAudio]) {
        const AudioStreamBasicDescription *format=nullptr;
        for(id desc in track.formatDescriptions) {
            format=CMAudioFormatDescriptionGetStreamBasicDescription((__bridge CMAudioFormatDescriptionRef)desc);
            if(format) break;
        }
        if(!format) continue;
        NSMutableArray *segments=[NSMutableArray new];
        for(AVAssetTrackSegment *segment in track.segments) {
            CMTimeMapping map=segment.timeMapping;
            [segments addObject:@{@"empty":@(segment.empty),@"source_start":finiteTime(map.source.start),
                @"source_duration":finiteTime(map.source.duration),@"target_start":finiteTime(map.target.start),
                @"target_duration":finiteTime(map.target.duration)}];
        }
        [rows addObject:@{@"id":@(track.trackID),@"channels":@(format->mChannelsPerFrame),
            @"rate":@(format->mSampleRate),@"start":finiteTime(track.timeRange.start),
            @"duration":finiteTime(track.timeRange.duration),@"segments":segments,
            @"language":track.extendedLanguageTag?:track.languageCode?:@"und"}];
    }
    double duration=CMTimeGetSeconds(asset.duration);
    return json(@{@"tracks":rows,@"duration":@(std::isfinite(duration)?duration:0.)});
}}
@interface RasteraAudioReader:NSObject
@property AVURLAsset *asset;
@property AVAssetReader *reader;
@property AVAssetReaderTrackOutput *output;
@property AVAssetTrack *track;
@property CMSampleBufferRef pending;
@property size_t cursor;
@end
@implementation RasteraAudioReader
-(void)dealloc {if(_pending)CFRelease(_pending);[_reader cancelReading];}
@end
static bool resetReader(RasteraAudioReader *reader,double time) {
    if(!std::isfinite(time)||time<0.) return false;
    if(reader.pending){CFRelease(reader.pending);reader.pending=nullptr;}
    reader.cursor=0;[reader.reader cancelReading];NSError *error=nil;
    reader.reader=[[AVAssetReader alloc]initWithAsset:reader.asset error:&error];
    if(!reader.reader) return false;
    // Omit channel count/layout: preserve every discrete channel in this exact track.
    reader.output=[[AVAssetReaderTrackOutput alloc]initWithTrack:reader.track outputSettings:@{
        AVFormatIDKey:@(kAudioFormatLinearPCM),AVLinearPCMBitDepthKey:@32,AVLinearPCMIsFloatKey:@YES,
        AVLinearPCMIsBigEndianKey:@NO,AVLinearPCMIsNonInterleaved:@NO,AVSampleRateKey:@48000}];
    reader.output.alwaysCopiesSampleData=NO;
    if(![reader.reader canAddOutput:reader.output]) return false;
    [reader.reader addOutput:reader.output];
    if(time>0.)reader.reader.timeRange=CMTimeRangeMake(CMTimeMakeWithSeconds(time,48000),kCMTimePositiveInfinity);
    return [reader.reader startReading];
}
extern "C" void *rastera_audio_reader_create(const char *path,int32_t trackID) { @autoreleasepool {
    RasteraAudioReader *reader=[RasteraAudioReader new];
    reader.asset=[AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:@(path)] options:nil];
    for(AVAssetTrack *track in [reader.asset tracksWithMediaType:AVMediaTypeAudio]) if(track.trackID==trackID)reader.track=track;
    if(!reader.track||!resetReader(reader,0.))return nullptr;
    return (__bridge_retained void*)reader;
}}
extern "C" bool rastera_audio_reader_seek(void *handle,double seconds) { @autoreleasepool {
    return resetReader((__bridge RasteraAudioReader*)handle,seconds);
}}
extern "C" int rastera_audio_reader_read(void *handle,float *out,uint32_t capacity,uint32_t *channels,double *pts) { @autoreleasepool {
    RasteraAudioReader *reader=(__bridge RasteraAudioReader*)handle;
    if(!reader.pending){reader.pending=[reader.output copyNextSampleBuffer];reader.cursor=0;}
    if(!reader.pending)return reader.reader.status==AVAssetReaderStatusFailed?-1:0;
    const AudioStreamBasicDescription *format=CMAudioFormatDescriptionGetStreamBasicDescription(CMSampleBufferGetFormatDescription(reader.pending));
    if(!format||format->mFormatID!=kAudioFormatLinearPCM||!(format->mFormatFlags&kAudioFormatFlagIsFloat)||
       format->mBitsPerChannel!=32||(format->mFormatFlags&kAudioFormatFlagIsBigEndian)||
       (format->mFormatFlags&kAudioFormatFlagIsNonInterleaved)||format->mSampleRate!=48000.||
       format->mChannelsPerFrame>32||!format->mChannelsPerFrame||format->mBytesPerFrame!=format->mChannelsPerFrame*sizeof(float))return -2;
    *channels=format->mChannelsPerFrame;
    size_t n=std::min((size_t)capacity,(size_t)CMSampleBufferGetNumSamples(reader.pending)-reader.cursor);
    CMBlockBufferRef data=CMSampleBufferGetDataBuffer(reader.pending);
    if(!data||CMBlockBufferCopyDataBytes(data,reader.cursor*(*channels)*sizeof(float),n*(*channels)*sizeof(float),out)!=noErr)return -3;
    double time=CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(reader.pending));
    if(!std::isfinite(time))return -4;
    *pts=time+(double)reader.cursor/48000.; reader.cursor+=n;
    if(reader.cursor==(size_t)CMSampleBufferGetNumSamples(reader.pending)){CFRelease(reader.pending);reader.pending=nullptr;}
    return (int)n;
}}
extern "C" void rastera_audio_reader_destroy(void *handle) { @autoreleasepool {
    RasteraAudioReader *reader=(__bridge_transfer RasteraAudioReader*)handle;(void)reader;
}}
namespace {
constexpr size_t capacity=8192,maximumChannels=32;
struct Frame {double host=0;std::array<float,maximumChannels>samples{};};
struct Ring {
    std::unique_ptr<Frame[]> frames{new Frame[capacity]};
    std::atomic<size_t>read{0},write{0};
    bool push(const Frame &frame) {
        size_t w=write.load(std::memory_order_relaxed),next=(w+1)%capacity;
        if(next==read.load(std::memory_order_acquire))return false;
        frames[w]=frame;write.store(next,std::memory_order_release);return true;
    }
    bool pop(Frame &frame) {
        size_t r=read.load(std::memory_order_relaxed);
        if(r==write.load(std::memory_order_acquire))return false;
        frame=frames[r];read.store((r+1)%capacity,std::memory_order_release);return true;
    }
};

// Apple's streaming sample-rate converter runs on the mixer worker, never the HAL IOProc.
// Normal priming requests lookahead and has no output timeline delay (AudioConverter.h).
constexpr OSStatus converterNeedsData=(OSStatus)0x6e646174; // temporary no data, not EOF
constexpr UInt32 conversionInputCapacity=1024,conversionOutputCapacity=8192;
struct RateConverter {
    AudioConverterRef handle=nullptr;UInt32 channels=0,readAhead=0,lastVended=0;
    std::array<std::unique_ptr<float[]>,2>input;
    std::unique_ptr<float[]>output;
    ~RateConverter(){if(handle)AudioConverterDispose(handle);}
    bool create(double rate,UInt32 count,double sourceRate=48000.) {
        channels=count;AudioStreamBasicDescription from{},to{};
        from.mSampleRate=sourceRate;from.mFormatID=kAudioFormatLinearPCM;from.mFormatFlags=kAudioFormatFlagIsFloat|kAudioFormatFlagIsPacked;
        from.mBitsPerChannel=32;from.mChannelsPerFrame=count;from.mFramesPerPacket=1;from.mBytesPerFrame=count*sizeof(float);from.mBytesPerPacket=from.mBytesPerFrame;
        to=from;to.mSampleRate=rate;
        if(AudioConverterNew(&from,&to,&handle)!=noErr)return false;
        UInt32 quality=kAudioConverterQuality_High,prime=kConverterPrimeMethod_Normal;
        if(AudioConverterSetProperty(handle,kAudioConverterSampleRateConverterQuality,sizeof(quality),&quality)!=noErr||
           AudioConverterSetProperty(handle,kAudioConverterPrimeMethod,sizeof(prime),&prime)!=noErr)return false;
        AudioConverterPrimeInfo info{};UInt32 bytes=sizeof(info);
        if(AudioConverterGetProperty(handle,kAudioConverterPrimeInfo,&bytes,&info)==noErr)readAhead=info.trailingFrames;
        for(auto &buffer:input)buffer.reset(new float[conversionInputCapacity*count]);
        output.reset(new float[conversionOutputCapacity*count]);
        // Warm lazy converter scratch/filter allocation on the control owner before use.
        std::array<float,256*maximumChannels>silence{};UInt32 warmed=0;
        if(convert(silence.data(),256,count,warmed)!=noErr)return false;
        bytes=sizeof(info);if(AudioConverterGetProperty(handle,kAudioConverterPrimeInfo,&bytes,&info)==noErr)readAhead=info.trailingFrames;
        return AudioConverterReset(handle)==noErr;
    }
    struct Feed{RateConverter *converter;UInt32 index,frames,cursor=0;};
    static OSStatus supply(AudioConverterRef,UInt32 *packets,AudioBufferList *data,AudioStreamPacketDescription**,void *context) {
        auto *f=(Feed*)context;UInt32 frames=std::min(*packets,f->frames-f->cursor);
        *packets=frames;data->mNumberBuffers=1;data->mBuffers[0].mNumberChannels=f->converter->channels;
        data->mBuffers[0].mDataByteSize=frames*f->converter->channels*sizeof(float);
        data->mBuffers[0].mData=frames?f->converter->input[f->index].get()+f->cursor*f->converter->channels:nullptr;
        f->converter->lastVended=f->index;f->cursor+=frames;return frames?noErr:converterNeedsData;
    }
    OSStatus convert(const float *source,UInt32 frames,UInt32 sourceChannels,UInt32 &produced) {
        if(frames>conversionInputCapacity)return kAudioConverterErr_InvalidInputSize;
        UInt32 next=1-lastVended;float *packed=input[next].get();
        for(UInt32 i=0;i<frames;i++)for(UInt32 c=0;c<channels;c++)packed[i*channels+c]=c<sourceChannels?source[i*sourceChannels+c]:0.;
        Feed feed{this,next,frames};AudioBufferList list{};list.mNumberBuffers=1;
        list.mBuffers[0]={channels,conversionOutputCapacity*channels*(UInt32)sizeof(float),output.get()};
        produced=conversionOutputCapacity;OSStatus status=AudioConverterFillComplexBuffer(handle,supply,&feed,&produced,&list,nullptr);
        if(feed.cursor!=frames&&status==noErr)return kAudioConverterErr_InvalidInputSize;
        return status==converterNeedsData?noErr:status;
    }
};
static std::atomic<bool>emergencyMute{true};
struct Device {
    AudioDeviceID id=0;AudioDeviceIOProcID proc=nullptr;Ring output,input;
    std::unique_ptr<RateConverter>converter,inputConverter;double conversionStart=0,expectedInputHost=0;uint64_t convertedFrames=0;
    std::array<float,128*maximumChannels>inputScratch{};UInt32 inputPendingCount=0,inputPendingCursor=0;
    double inputConversionStart=0,inputExpectedHost=0,inputPendingHost=0,inputRawClockCorrection=0;uint64_t inputConvertedFrames=0;
    std::atomic<uint64_t>inputConverterErrors{0},inputConverterResets{0},inputReturned48kFrames{0};
    std::atomic<uint64_t>converterErrors{0},converterResets{0},convertedOutputFrames{0};
    double rate=48000,tickSeconds=0;UInt32 outChannels=0,inChannels=0;bool capture=false;
    Frame a,b,readPending;bool haveA=false,haveB=false,haveReadPending=false;
    std::atomic_flag writer=ATOMIC_FLAG_INIT,reader=ATOMIC_FLAG_INIT;
    std::atomic<uint64_t>callbacks{0},underflow{0},overflow{0},inputOverflow{0},invalidTime{0};
    std::atomic<uint64_t>outputFrames{0},inputFrames{0},outputMissingHost{0},inputMissingHost{0},zeroHost{0},contentFrames{0},outputClipped{0},reentrantAccess{0};
    std::atomic<double>firstHost{0},lastHost{0},firstSample{0},lastSample{0},maxCallbackGap{0},outputPeak{0};
    std::atomic<double>intervalStart{0},intervalMaxGap{0};std::atomic<uint64_t>intervalMaxUpdateMisses{0};
    std::atomic<bool>clockObserved{false},formatChanged{false};
    std::atomic<uint64_t>clockSequence{0},clockDiscontinuities{0};
    std::atomic<double>inputFirstHost{0},inputLastHost{0},inputFirstSample{0},inputLastSample{0};
    std::atomic<uint64_t>inputClockSequence{0};std::atomic<bool>inputClockObserved{false};
    bool firstInputClock=true;
    double previousCallback=0;bool firstClock=true;
    std::array<AudioObjectPropertyAddress,5> listenerAddresses;
    std::array<bool,5>listenerAdded{};
};
static_assert(std::atomic<uint64_t>::is_always_lock_free&&std::atomic<double>::is_always_lock_free,"IO telemetry must be lock-free");
static UInt32 frameCount(const AudioBufferList *list) {
    if(!list||!list->mNumberBuffers)return 0;
    UInt32 frames=UINT32_MAX;
    for(UInt32 i=0;i<list->mNumberBuffers;i++) {
        const auto &b=list->mBuffers[i];
        if(b.mNumberChannels)frames=std::min(frames,b.mDataByteSize/(UInt32)(sizeof(float)*b.mNumberChannels));
    }
    return frames==UINT32_MAX?0:std::min(frames,(UInt32)capacity);
}
static OSStatus changed(AudioObjectID,UInt32,const AudioObjectPropertyAddress*,void *context) {
    ((Device*)context)->formatChanged.store(true,std::memory_order_release);return noErr;
}
static OSStatus io(AudioDeviceID,const AudioTimeStamp*,const AudioBufferList *input,const AudioTimeStamp *inputTime,
        AudioBufferList *output,const AudioTimeStamp *outputTime,void *context) {
    auto *d=(Device*)context;d->callbacks.fetch_add(1,std::memory_order_relaxed);
    const double now=(double)mach_absolute_time()*d->tickSeconds;
    if(d->previousCallback>0) {
        double gap=now-d->previousCallback;
        d->maxCallbackGap.store(std::max(d->maxCallbackGap.load(std::memory_order_relaxed),gap),std::memory_order_relaxed);
        double expected=d->intervalMaxGap.load(std::memory_order_relaxed);bool stored=gap<=expected;
        // A control snapshot may reset this maximum. At most two lock-free attempts in IO.
        for(int attempt=0;attempt<2&&!stored;attempt++)stored=d->intervalMaxGap.compare_exchange_strong(expected,std::max(expected,gap),std::memory_order_relaxed);
        if(!stored)d->intervalMaxUpdateMisses.fetch_add(1,std::memory_order_relaxed);
    }
    d->previousCallback=now;
    if(output)for(UInt32 i=0;i<output->mNumberBuffers;i++)if(output->mBuffers[i].mData)std::memset(output->mBuffers[i].mData,0,output->mBuffers[i].mDataByteSize);
    if(d->formatChanged.load(std::memory_order_acquire))return noErr;
    UInt32 n=frameCount(output);
    if(n) {
        const bool hostValid=outputTime&&(outputTime->mFlags&kAudioTimeStampHostTimeValid)&&outputTime->mHostTime!=0;
        const double host=hostValid?(double)outputTime->mHostTime*d->tickSeconds:now;
        if(!hostValid){d->invalidTime.fetch_add(1,std::memory_order_relaxed);d->outputMissingHost.fetch_add(1,std::memory_order_relaxed);}
        if(!outputTime||!outputTime->mHostTime)d->zeroHost.fetch_add(1,std::memory_order_relaxed);
        if(hostValid&&(outputTime->mFlags&kAudioTimeStampSampleTimeValid)&&std::isfinite(outputTime->mSampleTime)) {
            d->clockSequence.fetch_add(1,std::memory_order_acq_rel);
            if(!d->firstClock&&(host<=d->lastHost.load()||outputTime->mSampleTime<d->lastSample.load())){d->firstClock=true;d->clockDiscontinuities.fetch_add(1);}
            if(d->firstClock){d->firstHost.store(host);d->firstSample.store(outputTime->mSampleTime);d->firstClock=false;}
            d->lastSample.store(outputTime->mSampleTime);d->lastHost.store(host);d->clockObserved.store(true,std::memory_order_relaxed);
            d->clockSequence.fetch_add(1,std::memory_order_release);
        }
        d->outputFrames.fetch_add(n,std::memory_order_relaxed);
        double peak=d->outputPeak.load(std::memory_order_relaxed);
        for(UInt32 i=0;i<n;i++) {
            const double t=host+(double)i/d->rate;
            if(!d->haveA)d->haveA=d->output.pop(d->a);
            if(!d->haveB)d->haveB=d->output.pop(d->b);
            while(d->haveA&&d->haveB&&d->b.host<t){d->a=d->b;d->haveB=d->output.pop(d->b);}
            const bool ready=d->haveA&&d->haveB&&t>=d->a.host&&d->b.host>d->a.host&&d->b.host-d->a.host<1.5/d->rate;
            const double blend=ready?std::clamp((t-d->a.host)/(d->b.host-d->a.host),0.,1.):0.;
            if(!ready)d->underflow.fetch_add(1,std::memory_order_relaxed);
            UInt32 channel=0;bool content=false;
            for(UInt32 j=0;j<output->mNumberBuffers;j++) {
                AudioBuffer &buffer=output->mBuffers[j];float *samples=(float*)buffer.mData;
                for(UInt32 c=0;c<buffer.mNumberChannels;c++,channel++) {
                    float value=ready&&channel<maximumChannels?(float)(d->a.samples[channel]+blend*(d->b.samples[channel]-d->a.samples[channel])):0.;
                    if(!std::isfinite(value))value=0.;
                    if(emergencyMute.load(std::memory_order_relaxed))value=0.;
                    if(std::abs(value)>1.)d->outputClipped.fetch_add(1,std::memory_order_relaxed);
                    value=std::clamp(value,-1.f,1.f);peak=std::max(peak,(double)std::abs(value));content|=value!=0.;
                    if(samples)samples[i*buffer.mNumberChannels+c]=value;
                }
            }
            if(content)d->contentFrames.fetch_add(1,std::memory_order_relaxed);
        }
        d->outputPeak.store(peak,std::memory_order_relaxed);
    }
    n=d->capture?frameCount(input):0;
    if(n) {
        const bool valid=inputTime&&(inputTime->mFlags&kAudioTimeStampHostTimeValid)&&inputTime->mHostTime!=0;
        const double host=valid?(double)inputTime->mHostTime*d->tickSeconds:now;
        if(!valid){d->invalidTime.fetch_add(1,std::memory_order_relaxed);d->inputMissingHost.fetch_add(1,std::memory_order_relaxed);}
        if(!inputTime||!inputTime->mHostTime)d->zeroHost.fetch_add(1,std::memory_order_relaxed);
        d->inputFrames.fetch_add(n,std::memory_order_relaxed);
        if(valid&&(inputTime->mFlags&kAudioTimeStampSampleTimeValid)&&std::isfinite(inputTime->mSampleTime)) {
            d->inputClockSequence.fetch_add(1,std::memory_order_acq_rel);
            if(d->firstInputClock){d->inputFirstHost.store(host);d->inputFirstSample.store(inputTime->mSampleTime);d->firstInputClock=false;}
            d->inputLastHost.store(host);d->inputLastSample.store(inputTime->mSampleTime);d->inputClockObserved.store(true,std::memory_order_relaxed);
            d->inputClockSequence.fetch_add(1,std::memory_order_release);
        }
        for(UInt32 i=0;i<n;i++) {
            Frame frame;frame.host=host+(double)i/d->rate;UInt32 channel=0;
            for(UInt32 j=0;j<input->mNumberBuffers;j++) {
                const AudioBuffer &buffer=input->mBuffers[j];const float *samples=(const float*)buffer.mData;
                for(UInt32 c=0;c<buffer.mNumberChannels;c++,channel++)
                    if(channel<maximumChannels&&samples)frame.samples[channel]=samples[i*buffer.mNumberChannels+c];
            }
            if(!d->input.push(frame))d->inputOverflow.fetch_add(1,std::memory_order_relaxed);
        }
    }
    return noErr;
}
static bool floatStreams(AudioDeviceID id,AudioObjectPropertyScope scope,double rate) {
    auto a=address(kAudioDevicePropertyStreams,scope);UInt32 bytes=0;
    if(AudioObjectGetPropertyDataSize(id,&a,0,nullptr,&bytes)!=noErr)return false;
    std::unique_ptr<AudioStreamID[]> streams(new AudioStreamID[bytes/sizeof(AudioStreamID)]);
    if(AudioObjectGetPropertyData(id,&a,0,nullptr,&bytes,streams.get())!=noErr)return false;
    for(UInt32 i=0;i<bytes/sizeof(AudioStreamID);i++) {
        AudioStreamBasicDescription format{};
        if(!property(streams[i],kAudioStreamPropertyVirtualFormat,format)||format.mFormatID!=kAudioFormatLinearPCM||
           !(format.mFormatFlags&kAudioFormatFlagIsFloat)||(format.mFormatFlags&kAudioFormatFlagIsBigEndian)||
           format.mBitsPerChannel!=32||format.mSampleRate!=rate||!format.mChannelsPerFrame)return false;
        UInt32 packedChannels=(format.mFormatFlags&kAudioFormatFlagIsNonInterleaved)?1:format.mChannelsPerFrame;
        if(format.mBytesPerFrame!=packedChannels*sizeof(float))return false;
    }
    return true;
}
}
extern "C" void *rastera_audio_device_open(const char *uid,bool capture) { @autoreleasepool {
    std::unique_ptr<Device>d(new Device);UInt32 matches=0;
    for(NSNumber *number in deviceIDs())if([stringProperty(number.unsignedIntValue,kAudioDevicePropertyDeviceUID)isEqualToString:@(uid)]){d->id=number.unsignedIntValue;matches++;}
    if(matches!=1)return nullptr;
    UInt32 alive=0;if(!property(d->id,kAudioDevicePropertyDeviceIsAlive,alive)||!alive)return nullptr;
    if(!property(d->id,kAudioDevicePropertyNominalSampleRate,d->rate)||!std::isfinite(d->rate)||d->rate<8000||d->rate>192000)return nullptr;
    d->outChannels=channels(d->id,kAudioDevicePropertyScopeOutput);d->inChannels=channels(d->id,kAudioDevicePropertyScopeInput);
    if(d->outChannels>maximumChannels||d->inChannels>maximumChannels||(!d->outChannels&&!capture)||
       (capture&&!d->inChannels)||(d->outChannels&&!floatStreams(d->id,kAudioDevicePropertyScopeOutput,d->rate))||
       (capture&&!floatStreams(d->id,kAudioDevicePropertyScopeInput,d->rate)))return nullptr;
    if(capture&&[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]!=AVAuthorizationStatusAuthorized)return nullptr;
    mach_timebase_info_data_t timebase;mach_timebase_info(&timebase);
    d->tickSeconds=(double)timebase.numer/timebase.denom/1e9;d->capture=capture;
    d->intervalStart.store((double)mach_absolute_time()*d->tickSeconds);
    if(d->outChannels&&d->rate!=48000.){d->converter=std::make_unique<RateConverter>();if(!d->converter->create(d->rate,d->outChannels))return nullptr;}
    if(capture&&d->rate!=48000.){d->inputConverter=std::make_unique<RateConverter>();if(!d->inputConverter->create(48000.,d->inChannels,d->rate))return nullptr;}
    if(AudioDeviceCreateIOProcID(d->id,io,d.get(),&d->proc)!=noErr)return nullptr;
    // HAL IOProcs otherwise enable every stream on a duplex device. Explicitly turn
    // input streams OFF for output-only patches; merely ignoring input buffers is insufficient.
    if(d->inChannels) {
        auto streamsAddress=address(kAudioDevicePropertyStreams,kAudioDevicePropertyScopeInput);UInt32 streamBytes=0;
        if(AudioObjectGetPropertyDataSize(d->id,&streamsAddress,0,nullptr,&streamBytes)!=noErr){AudioDeviceDestroyIOProcID(d->id,d->proc);return nullptr;}
        UInt32 count=streamBytes/sizeof(AudioStreamID);
        UInt32 usageBytes=(UInt32)(offsetof(AudioHardwareIOProcStreamUsage,mStreamIsOn)+count*sizeof(UInt32));
        std::unique_ptr<uint8_t[]> memory(new uint8_t[usageBytes]);std::memset(memory.get(),0,usageBytes);
        auto *usage=(AudioHardwareIOProcStreamUsage*)memory.get();usage->mIOProc=(void*)d->proc;usage->mNumberStreams=count;
        for(UInt32 i=0;i<count;i++)usage->mStreamIsOn[i]=capture?1:0;
        auto usageAddress=address(kAudioDevicePropertyIOProcStreamUsage,kAudioDevicePropertyScopeInput);
        if(AudioObjectSetPropertyData(d->id,&usageAddress,0,nullptr,usageBytes,usage)!=noErr||
           AudioObjectGetPropertyData(d->id,&usageAddress,0,nullptr,&usageBytes,usage)!=noErr){AudioDeviceDestroyIOProcID(d->id,d->proc);return nullptr;}
        for(UInt32 i=0;i<count;i++)if((usage->mStreamIsOn[i]!=0)!=capture){AudioDeviceDestroyIOProcID(d->id,d->proc);return nullptr;}
    }
    d->listenerAddresses={address(kAudioDevicePropertyNominalSampleRate),address(kAudioDevicePropertyBufferFrameSize),
        address(kAudioDevicePropertyDeviceIsAlive),address(kAudioDevicePropertyStreamConfiguration,kAudioDevicePropertyScopeInput),
        address(kAudioDevicePropertyStreamConfiguration,kAudioDevicePropertyScopeOutput)};
    for(size_t i=0;i<d->listenerAddresses.size();i++)d->listenerAdded[i]=AudioObjectAddPropertyListener(d->id,&d->listenerAddresses[i],changed,d.get())==noErr;
    if(AudioDeviceStart(d->id,d->proc)!=noErr){for(size_t i=0;i<d->listenerAddresses.size();i++)if(d->listenerAdded[i])AudioObjectRemovePropertyListener(d->id,&d->listenerAddresses[i],changed,d.get());AudioDeviceDestroyIOProcID(d->id,d->proc);return nullptr;}
    return d.release();
}}
extern "C" void rastera_audio_device_close(void *handle) {
    auto *d=(Device*)handle;AudioDeviceStop(d->id,d->proc);
    for(size_t i=0;i<d->listenerAddresses.size();i++)if(d->listenerAdded[i])AudioObjectRemovePropertyListener(d->id,&d->listenerAddresses[i],changed,d);
    AudioDeviceDestroyIOProcID(d->id,d->proc);delete d;
}
extern "C" uint32_t rastera_audio_device_write(void *handle,double host,const float *data,uint32_t frames,uint32_t channels) {
    auto *d=(Device*)handle;if(channels>maximumChannels||!channels||frames>conversionInputCapacity||!std::isfinite(host))return 0;
    if(d->writer.test_and_set(std::memory_order_acquire)){d->reentrantAccess.fetch_add(1);return 0;}
    UInt32 count=frames;const float *pcm=data;UInt32 stride=channels;double start=host;
    if(d->converter) {
        if(!d->expectedInputHost||std::abs(host-d->expectedInputHost)>0.5/48000.) {
            if(d->expectedInputHost){AudioConverterReset(d->converter->handle);d->converterResets.fetch_add(1);}
            d->conversionStart=host;d->convertedFrames=0;
        }
        d->expectedInputHost=host+(double)frames/48000.;
        OSStatus status=d->converter->convert(data,frames,channels,count);
        if(status!=noErr){d->converterErrors.fetch_add(1);d->writer.clear(std::memory_order_release);return 0;}
        pcm=d->converter->output.get();stride=d->outChannels;start=d->conversionStart+(double)d->convertedFrames/d->rate;
        d->convertedFrames+=count;
    }
    uint32_t written=0;
    for(uint32_t i=0;i<count;i++) {
        Frame frame;frame.host=start+(double)i/d->rate;
        for(uint32_t c=0;c<stride;c++)frame.samples[c]=pcm[i*stride+c];
        if(d->output.push(frame))written++;else d->overflow.fetch_add(1,std::memory_order_relaxed);
    }
    d->convertedOutputFrames.fetch_add(written,std::memory_order_relaxed);
    d->writer.clear(std::memory_order_release);return written==count?frames:0;
}
static uint32_t readRaw(Device *d,double *host,float *data,uint32_t frames,uint32_t channels) {
    Frame frame;uint32_t n=0;
    while(n<frames) {
        if(d->haveReadPending){frame=d->readPending;d->haveReadPending=false;}else if(!d->input.pop(frame))break;
        if(!n)*host=frame.host;
        else if(std::abs(frame.host-(*host+(double)n/d->rate))>0.5/d->rate){d->readPending=frame;d->haveReadPending=true;break;}
        for(uint32_t c=0;c<channels;c++)data[n*channels+c]=frame.samples[c];n++;
    }
    return n;
}
extern "C" uint32_t rastera_audio_device_read_48k(void *handle,double *host,float *data,uint32_t frames,uint32_t channels) {
    auto *d=(Device*)handle;if(channels>maximumChannels||!channels||channels>d->inChannels||frames>128)return 0;
    if(d->reader.test_and_set(std::memory_order_acquire)){d->reentrantAccess.fetch_add(1);return 0;}
    if(!d->inputConverter){UInt32 n=readRaw(d,host,data,frames,channels);d->inputReturned48kFrames.fetch_add(n);d->reader.clear(std::memory_order_release);return n;}
    UInt32 n=0;int iterations=0;
    while(n<frames&&iterations++<16) {
        if(d->inputPendingCursor==d->inputPendingCount) {
            double rawHost=0;UInt32 raw=readRaw(d,&rawHost,d->inputScratch.data(),128,d->inChannels);if(!raw)break;
            if(!d->inputExpectedHost||std::abs(rawHost-d->inputExpectedHost)>0.5/d->rate) {
                if(d->inputExpectedHost){AudioConverterReset(d->inputConverter->handle);d->inputConverterResets.fetch_add(1);}
                d->inputConversionStart=rawHost;d->inputConvertedFrames=0;d->inputRawClockCorrection=0;
            }else d->inputRawClockCorrection+=rawHost-d->inputExpectedHost;
            d->inputExpectedHost=rawHost+(double)raw/d->rate;
            d->inputPendingHost=d->inputConversionStart+(double)d->inputConvertedFrames/48000.+d->inputRawClockCorrection;
            d->inputPendingCount=0;d->inputPendingCursor=0;
            OSStatus status=d->inputConverter->convert(d->inputScratch.data(),raw,d->inChannels,d->inputPendingCount);
            if(status!=noErr){d->inputConverterErrors.fetch_add(1);break;}
            d->inputConvertedFrames+=d->inputPendingCount;
        }
        if(d->inputPendingCursor==d->inputPendingCount)continue;
        double time=d->inputPendingHost+(double)d->inputPendingCursor/48000.;
        if(!n)*host=time;else if(std::abs(time-(*host+(double)n/48000.))>0.5/48000.)break;
        UInt32 count=std::min(frames-n,d->inputPendingCount-d->inputPendingCursor);
        for(UInt32 i=0;i<count;i++)for(UInt32 c=0;c<channels;c++)data[(n+i)*channels+c]=d->inputConverter->output[(d->inputPendingCursor+i)*d->inChannels+c];
        d->inputPendingCursor+=count;n+=count;
    }
    d->inputReturned48kFrames.fetch_add(n);d->reader.clear(std::memory_order_release);return n;
}
extern "C" void rastera_audio_device_stats(void *handle,uint64_t *values) {
    auto *d=(Device*)handle;values[0]=d->callbacks.load();values[1]=d->underflow.load();values[2]=d->overflow.load();
    values[3]=d->inputOverflow.load();values[4]=d->invalidTime.load();values[5]=(d->output.write.load()+capacity-d->output.read.load())%capacity;
}
extern "C" char *rastera_audio_device_details(void *handle) { @autoreleasepool {
    auto *d=(Device*)handle;bool valid=false,coherent=false;double first=0,last=0,firstSample=0,lastSample=0;
    for(int tries=0;tries<2&&!coherent;tries++){auto seq=d->clockSequence.load(std::memory_order_acquire);if(seq&1)continue;
        valid=d->clockObserved.load();first=d->firstHost.load();last=d->lastHost.load();firstSample=d->firstSample.load();lastSample=d->lastSample.load();
        coherent=d->clockSequence.load(std::memory_order_acquire)==seq;}
    id ppm=coherent&&valid&&last>first?(id)@(((lastSample-firstSample)/(last-first)/d->rate-1.)*1e6):(id)NSNull.null;
    bool inputCoherent=false,inputValid=false;double inFirst=0,inLast=0,inFirstSample=0,inLastSample=0;
    for(int tries=0;tries<2&&!inputCoherent;tries++){auto seq=d->inputClockSequence.load(std::memory_order_acquire);if(seq&1)continue;
        inputValid=d->inputClockObserved.load();inFirst=d->inputFirstHost.load();inLast=d->inputLastHost.load();inFirstSample=d->inputFirstSample.load();inLastSample=d->inputLastSample.load();
        inputCoherent=d->inputClockSequence.load(std::memory_order_acquire)==seq;}
    id inputPPM=inputCoherent&&inputValid&&inLast>inFirst?(id)@(((inLastSample-inFirstSample)/(inLast-inFirst)/d->rate-1.)*1e6):(id)NSNull.null;
    double intervalEnd=CACurrentMediaTime(),intervalStart=d->intervalStart.exchange(intervalEnd),intervalGap=d->intervalMaxGap.exchange(0.);
    return json(@{@"interval_start_host":@(intervalStart),@"interval_end_host":@(intervalEnd),
        @"interval_max_callback_gap_seconds":@(intervalGap),@"interval_max_update_misses":@(d->intervalMaxUpdateMisses.load()),
        @"output_frames":@(d->outputFrames.load()),@"input_frames":@(d->inputFrames.load()),
        @"content_frames":@(d->contentFrames.load()),@"output_peak":@(d->outputPeak.load()),@"output_clipped_samples":@(d->outputClipped.load()),
        @"output_missing_host_timestamps":@(d->outputMissingHost.load()),@"input_missing_host_timestamps":@(d->inputMissingHost.load()),
        @"zero_host_timestamps":@(d->zeroHost.load()),@"first_host":@(first),@"last_host":@(last),@"first_device_sample":@(firstSample),
        @"last_device_sample":@(lastSample),@"measured_clock_ppm":ppm,@"clock_observation_coherent":@(coherent),
        @"clock_discontinuities":@(d->clockDiscontinuities.load()),@"input_measured_clock_ppm":inputPPM,@"input_clock_observation_coherent":@(inputCoherent),@"max_callback_gap_seconds":@(d->maxCallbackGap.load()),
        @"converter_errors":@(d->converterErrors.load()),@"converter_resets":@(d->converterResets.load()),
        @"converted_output_frames":@(d->convertedOutputFrames.load()),@"converter_read_ahead_48k_frames":@(d->converter?d->converter->readAhead:0),
        @"input_converter_errors":@(d->inputConverterErrors.load()),@"input_converter_resets":@(d->inputConverterResets.load()),
        @"input_returned_48k_frames":@(d->inputReturned48kFrames.load()),@"input_converter_read_ahead_device_frames":@(d->inputConverter?d->inputConverter->readAhead:0),
        @"format_changed":@(d->formatChanged.load()),@"capture":@(d->capture),@"reentrant_access":@(d->reentrantAccess.load())});
}}
extern "C" double rastera_audio_host_time(){return CACurrentMediaTime();}
extern "C" void rastera_audio_free_string(char *value){free(value);}
extern "C" void rastera_audio_emergency(bool mute){emergencyMute.store(mute,std::memory_order_release);}
extern "C" bool rastera_audio_is_muted(){return emergencyMute.load(std::memory_order_acquire);}
extern "C" void rastera_audio_shortcut() {
    NSCAssert(NSThread.isMainThread,@"App scoped shortcut registration on main");static id monitor=nil;
    if(!monitor)monitor=[NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown handler:^NSEvent*(NSEvent *event){
        NSEventModifierFlags f=event.modifierFlags&NSEventModifierFlagDeviceIndependentFlagsMask;
        if((f&(NSEventModifierFlagCommand|NSEventModifierFlagShift))==(NSEventModifierFlagCommand|NSEventModifierFlagShift)&&
           [[event.charactersIgnoringModifiers lowercaseString]isEqualToString:@"m"])emergencyMute.store(true,std::memory_order_release);
        return event;}];
}

// Offline diagnostic uses the identical converter and bounded block supply path as device writes.
extern "C" char *rastera_audio_resample_measure(double rate,double hz,bool impulse){@autoreleasepool{
    RateConverter converter;if(!std::isfinite(rate)||rate<8000||rate>192000||!converter.create(rate,2))return nullptr;
    std::array<float,128*2>input{};uint64_t producedTotal=0;double power=0,projectionR=0,projectionI=0,peak=0;uint64_t measured=0,peakFrame=0;uint32_t errors=0;
    constexpr UInt32 total=96000;double alias=hz>rate/2.?std::abs(rate-hz):hz;
    for(UInt32 offset=0;offset<total;offset+=128){for(UInt32 i=0;i<128;i++){float value=impulse?(offset+i==24000?1.f:0.f):(float)(.1*std::sin(2.*M_PI*hz*(offset+i)/48000.));input[i*2]=value;input[i*2+1]=value;}UInt32 count=0;OSStatus status=converter.convert(input.data(),128,2,count);if(status!=noErr)errors++;
    for(UInt32 i=0;i<count;i++){double v=converter.output[i*2],index=producedTotal+i;if(std::abs(v)>peak){peak=std::abs(v);peakFrame=(uint64_t)index;}if(index>=rate/2.){power+=v*v;double angle=2.*M_PI*alias*index/rate;projectionR+=v*std::cos(angle);projectionI+=v*std::sin(angle);measured++;}}producedTotal+=count;}
    return json(@{@"rate":@(rate),@"hz":@(hz),@"impulse":@(impulse),@"produced_frames":@(producedTotal),@"rms":@(measured?std::sqrt(power/measured):0.),@"tone_amplitude":@(measured?2.*std::hypot(projectionR,projectionI)/measured:0.),@"peak":@(peak),@"peak_frame":@(peakFrame),@"expected_impulse_frame":@(rate/2.),@"read_ahead_48k_frames":@(converter.readAhead),@"errors":@(errors)});
}}
