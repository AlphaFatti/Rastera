// Diagnostic control only. No wgpu handles or Rastera canvas resources cross this bridge.
#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>
#include <atomic>
#include <cmath>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>

namespace {
struct Event {
    const char *kind;
    uint64_t seq;
    double arrival;
    double a, b, c;
    uint64_t content, drawable;
};
struct Stats {
    std::mutex mutex;
    std::deque<Event> events;
    std::atomic<uint64_t> ticks{0}, submitted{0}, completed{0}, presented{0};
    std::atomic<uint64_t> zero{0}, unavailable{0}, backpressure{0}, lost{0}, errors{0};
    std::atomic<unsigned> inflight{0}, maxInflight{0};
    std::atomic<bool> stop{false};
    void record(const char *kind, uint64_t seq, double arrival, double a=0, double b=0, double c=0, uint64_t content=0, uint64_t drawable=0) {
        std::lock_guard<std::mutex> lock(mutex);
        if (events.size() == 4096) { events.pop_front(); ++lost; }
        events.push_back({kind,seq,arrival,a,b,c,content,drawable});
    }
};
using StatsHandle = std::shared_ptr<Stats>;
void report(const StatsHandle &stats) {
    std::deque<Event> rows;
    { std::lock_guard<std::mutex> lock(stats->mutex); rows.swap(stats->events); }
    // Only the reporting thread writes per-frame records, never display/Metal callbacks.
    for (const auto &e: rows) {
        const auto seq = (unsigned long long)e.seq;
        if (e.content) std::fprintf(stderr,"native_content_link kind=%s seq=%llu content_id=%llu drawable_id=%llu host_s=%.9f animation_s=%.9f\n",e.kind,seq,(unsigned long long)e.content,(unsigned long long)e.drawable,e.arrival,std::strcmp(e.kind,"content")==0?e.a:-1.0);
        if (std::strcmp(e.kind,"content")==0) continue;
        if (std::strcmp(e.kind,"schedule")==0)
            std::fprintf(stderr,"native_event kind=schedule seq=%llu callback_host_s=%.9f deadline_host_s=%.9f target_present_estimate_host_s=%.9f\n",seq,e.arrival,e.a,e.b);
        else if (std::strcmp(e.kind,"commit")==0)
            std::fprintf(stderr,"native_event kind=commit seq=%llu return_host_s=%.9f call_us=%.3f prepare_us=%.3f\n",seq,e.arrival,e.a,e.b);
        else if (std::strcmp(e.kind,"complete")==0)
            std::fprintf(stderr,"native_event kind=complete seq=%llu callback_host_s=%.9f gpu_start_host_s=%.9f gpu_end_host_s=%.9f status=%.0f\n",seq,e.arrival,e.a,e.b,e.c);
        else if (std::strcmp(e.kind,"presented")==0)
            std::fprintf(stderr,"native_event kind=presented_handler seq=%llu callback_host_s=%.9f presented_host_s=%.9f drawable_id=%.0f timestamp_valid=%.0f\n",seq,e.arrival,e.a,e.b,e.c);
        else if (std::strcmp(e.kind,"present_call")==0)
            std::fprintf(stderr,"native_event kind=present_call seq=%llu return_host_s=%.9f call_us=%.3f\n",seq,e.arrival,e.a);
        else
            std::fprintf(stderr,"native_event kind=presented_unavailable seq=%llu callback_host_s=%.9f timestamp_available=0\n",seq,e.arrival);
    }
    std::fprintf(stderr,"native_interval host_s=%.9f ticks=%llu submitted=%llu completed=%llu presented_callbacks=%llu zero_time=%llu unavailable_time=%llu backpressure=%llu lost_events=%llu errors=%llu inflight=%u max_inflight=%u\n",
        CACurrentMediaTime(),(unsigned long long)stats->ticks.load(),(unsigned long long)stats->submitted.load(),
        (unsigned long long)stats->completed.load(),(unsigned long long)stats->presented.load(),
        (unsigned long long)stats->zero.load(),(unsigned long long)stats->unavailable.load(),
        (unsigned long long)stats->backpressure.load(),(unsigned long long)stats->lost.load(),
        (unsigned long long)stats->errors.load(),stats->inflight.load(),stats->maxInflight.load());
}
}

@interface RasteraMetalView : NSView
- (void)updateDrawableSize;
@end
@implementation RasteraMetalView
- (CALayer *)makeBackingLayer { return [CAMetalLayer layer]; }
- (BOOL)wantsUpdateLayer { return YES; }
- (void)updateDrawableSize {
    NSCAssert([NSThread isMainThread], @"AppKit view operations require main thread");
    CAMetalLayer *metal = (CAMetalLayer *)self.layer;
    if (!metal) return;
    NSSize size = [self convertRectToBacking:self.bounds].size;
    if (size.width > 0 && size.height > 0) {
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        metal.contentsScale = self.window ? self.window.backingScaleFactor : 1.0;
        metal.drawableSize = CGSizeMake(size.width,size.height);
        [CATransaction commit];
        std::fprintf(stderr,"native_resize host_s=%.9f width=%.0f height=%.0f main_thread=1\n",CACurrentMediaTime(),size.width,size.height);
    }
}
- (void)setFrameSize:(NSSize)size { [super setFrameSize:size]; [self updateDrawableSize]; }
- (void)viewDidChangeBackingProperties { [super viewDidChangeBackingProperties]; [self updateDrawableSize]; }
- (void)viewDidMoveToWindow { [super viewDidMoveToWindow]; [self updateDrawableSize]; }
@end

API_AVAILABLE(macos(14.0))
@interface RasteraNativePresenter : NSObject <NSWindowDelegate, CAMetalDisplayLinkDelegate> {
@public
    StatsHandle stats;
    NSWindow *window;
@private
    CAMetalLayer *_metalLayer;
    id<MTLDevice> _device;
    id<MTLCommandQueue> _queue;
    id<MTLRenderPipelineState> _pipeline;
    dispatch_semaphore_t _slots;
    dispatch_group_t _gpuWork;
    std::thread _worker;
    BOOL _closed;
    std::chrono::steady_clock::time_point _animationOrigin;
    uint64_t _contentID;
}
- (BOOL)start:(BOOL)disableAnimations;
- (void)stop;
- (BOOL)isOpen;
@end

@implementation RasteraNativePresenter
- (BOOL)start:(BOOL)disableAnimations {
    NSCAssert([NSThread isMainThread], @"Create native window on main thread");
    stats = std::make_shared<Stats>();
    _slots = dispatch_semaphore_create(2);
    _gpuWork = dispatch_group_create();
    _device = MTLCreateSystemDefaultDevice();
    _queue = [_device newCommandQueue];
    if (!_device || !_queue) return NO;
    // Runtime Metal compilation uses the installed system compiler; no downloaded SDK.
    NSString *source = [NSString stringWithUTF8String:R"METAL(
#include <metal_stdlib>
using namespace metal;
vertex float4 diagnostic_vertex(uint i [[vertex_id]]) {
    const float2 p[3]={float2(-1,-1),float2(3,-1),float2(-1,3)};
    return float4(p[i],0,1);
}
struct Uniforms { float4 animation; uint4 identity; };
fragment float4 diagnostic_fragment(float4 p [[position]], constant Uniforms &data [[buffer(0)]]) {
    float4 u=data.animation;
    float2 uv=p.xy/u.yz;
    float3 color=uv.x<0.5 ? float3(0.03,0.16,0.50) : float3(0.50,0.08,0.02);
    if (fmod(p.x,80.0)<2.0 || fmod(p.y,80.0)<2.0) color=float3(0.6);
    float marker=0.08+fmod(u.x*0.25,0.84);
    if (abs(uv.x-marker)<0.015 && abs(uv.y-0.5)<0.2) color=float3(1,1,0);
    if (uv.y<0.08) color=uint(u.x*4.0)%2u ? float3(0,1,0) : float3(1,0,1);
    // Ten decimal digits encode the low 32 bits of the native content ID.
    if (uv.y>0.10 && uv.y<0.32 && uv.x>0.05 && uv.x<0.95) {
        float2 q=float2((uv.x-0.05)/0.09,(uv.y-0.10)/0.22);
        uint column=min(uint(q.x),9u), divisor=1u;
        for (uint i=column;i<9u;++i) divisor*=10u;
        uint digit=(data.identity.x/divisor)%10u;
        const uint masks[10]={63,6,91,79,102,109,125,7,127,111};
        float2 d=float2(fract(q.x),q.y);
        bool horizontal=d.x>0.18 && d.x<0.78;
        uint segment=7u;
        if(horizontal && d.y<0.13) segment=0;
        else if(horizontal && d.y>0.87) segment=3;
        else if(horizontal && abs(d.y-0.5)<0.065) segment=6;
        else if(d.x>0.72 && d.x<0.86) segment=d.y<0.5?1u:2u;
        else if(d.x>0.10 && d.x<0.24) segment=d.y<0.5?5u:4u;
        color=segment<7u && (masks[digit]&(1u<<segment)) ? float3(1) : float3(0);
    }
    // Machine-readable 32-bit stripe, least significant bit on the left.
    if(uv.y>0.82 && uv.y<0.92 && uv.x>0.02 && uv.x<0.98) {
        uint bit=min(uint((uv.x-0.02)/0.96*32.0),31u);
        color=(data.identity.x&(1u<<bit)) ? float3(1) : float3(0);
    }
    return float4(color,1);
}
)METAL"];
    NSError *error = nil;
    id<MTLLibrary> library = [_device newLibraryWithSource:source options:nil error:&error];
    if (!library) { NSLog(@"Native shader failure: %@",error); return NO; }
    MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
    desc.vertexFunction = [library newFunctionWithName:@"diagnostic_vertex"];
    desc.fragmentFunction = [library newFunctionWithName:@"diagnostic_fragment"];
    desc.colorAttachments[0].pixelFormat = MTLPixelFormatBGRA8Unorm;
    _pipeline = [_device newRenderPipelineStateWithDescriptor:desc error:&error];
    if (!_pipeline) { NSLog(@"Native pipeline failure: %@",error); return NO; }
    window = [[NSWindow alloc] initWithContentRect:NSMakeRect(20,150,600,338)
        styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable
        backing:NSBackingStoreBuffered defer:NO];
    window.title = @"Rastera — Native clean output (control)";
    if (disableAnimations) window.animationBehavior = NSWindowAnimationBehaviorNone;
    std::fprintf(stderr,"animation_settings host_s=%.9f window=native_output requested=%s readback=%ld reduce_motion=%d main_thread=1\n",
        CACurrentMediaTime(),disableAnimations?"none":"default",(long)window.animationBehavior,
        NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion);
    window.releasedWhenClosed = NO;
    window.delegate = self;
    RasteraMetalView *view = [[RasteraMetalView alloc] initWithFrame:NSMakeRect(0,0,600,338)];
    view.wantsLayer = YES;
    _metalLayer = (CAMetalLayer *)view.layer;
    _metalLayer.device = _device;
    _metalLayer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    _metalLayer.framebufferOnly = YES;
    _metalLayer.maximumDrawableCount = 3;
    _metalLayer.presentsWithTransaction = NO;
    _metalLayer.displaySyncEnabled = YES;
    _metalLayer.allowsNextDrawableTimeout = YES;
    window.contentView = view;
    [window makeKeyAndOrderFront:nil];
    [view updateDrawableSize];
    [window displayIfNeeded];
    std::fprintf(stderr,"native_start host_s=%.9f device=%s thread=dedicated gpu_limit=2 drawable_pool=3\n",CACurrentMediaTime(),_device.name.UTF8String);
    std::fprintf(stderr,"native_clock_anchor host_s=%.9f unix_us=%lld\n",CACurrentMediaTime(),(long long)std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::system_clock::now().time_since_epoch()).count());
    RasteraNativePresenter *presenter = self;
    _worker = std::thread([presenter] {
        @autoreleasepool {
            NSCAssert(![NSThread isMainThread],@"Output renderer must be independent of main thread");
            presenter->_animationOrigin = std::chrono::steady_clock::now();
            std::fprintf(stderr,"native_renderer_ready host_s=%.9f main_thread=0\n",CACurrentMediaTime());
            CAMetalDisplayLink *link = [[CAMetalDisplayLink alloc] initWithMetalLayer:presenter->_metalLayer];
            link.delegate = presenter;
            link.preferredFrameLatency = 2;
            link.preferredFrameRateRange = CAFrameRateRangeMake(60,60,60);
            [link addToRunLoop:NSRunLoop.currentRunLoop forMode:NSDefaultRunLoopMode];
            while (!presenter->stats->stop.load()) {
                @autoreleasepool { CFRunLoopRunInMode(kCFRunLoopDefaultMode,0.05,false); }
            }
            [link invalidate]; // Same thread/run loop as registration.
            link.delegate = nil;
        }
    });
    return YES;
}
- (void)metalDisplayLink:(CAMetalDisplayLink *)link needsUpdate:(CAMetalDisplayLinkUpdate *)update {
    @autoreleasepool {
        NSCAssert(![NSThread isMainThread],@"Display-link callback must stay on worker run loop");
        auto state = stats; // Callbacks retain C++ state, never a view or window.
        if (state->stop.load()) return;
        const double now = CACurrentMediaTime();
        const uint64_t seq = ++state->ticks;
        state->record("schedule",seq,now,update.targetTimestamp,update.targetPresentationTimestamp);
        if (dispatch_semaphore_wait(_slots,DISPATCH_TIME_NOW) != 0) { ++state->backpressure; return; }
        id<CAMetalDrawable> drawable = update.drawable; // Never ask nextDrawable on the UI thread.
        id<MTLCommandBuffer> command = [_queue commandBuffer];
        if (!drawable || !command) { ++state->errors; dispatch_semaphore_signal(_slots); return; }
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = drawable.texture;
        pass.colorAttachments[0].loadAction = MTLLoadActionClear;
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0,0,0,1);
        id<MTLRenderCommandEncoder> encoder = [command renderCommandEncoderWithDescriptor:pass];
        if (!encoder) { ++state->errors; dispatch_semaphore_signal(_slots); return; }
        [encoder setRenderPipelineState:_pipeline];
        const uint64_t content = ++_contentID;
        const uint64_t drawableID = drawable.drawableID;
        const double animation = std::chrono::duration<double>(std::chrono::steady_clock::now()-_animationOrigin).count();
        struct alignas(16) Uniforms { float animation[4]; uint32_t identity[4]; };
        const Uniforms uniform = {{float(fmod(animation,3600.0)),float(drawable.texture.width),float(drawable.texture.height),0},
                                  {uint32_t(content),uint32_t(content>>32),0,0}};
        state->record("content",seq,now,animation,0,0,content,drawableID);
        [encoder setFragmentBytes:&uniform length:sizeof(uniform) atIndex:0];
        [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
        [encoder endEncoding];
        unsigned active = ++state->inflight;
        unsigned max = state->maxInflight.load();
        while (active>max && !state->maxInflight.compare_exchange_weak(max,active)) {}
        dispatch_semaphore_t slots = _slots;
        dispatch_group_t work = _gpuWork;
        dispatch_group_enter(work);
        [command addCompletedHandler:^(id<MTLCommandBuffer> completed) {
            ++state->completed;
            if (completed.status == MTLCommandBufferStatusError) { ++state->errors; state->stop = true; }
            state->record("complete",seq,CACurrentMediaTime(),completed.GPUStartTime,completed.GPUEndTime,double(completed.status),content,drawableID);
            --state->inflight;
            dispatch_semaphore_signal(slots);
            dispatch_group_leave(work);
        }];
        if ([drawable respondsToSelector:@selector(addPresentedHandler:)]) {
            [drawable addPresentedHandler:^(id<MTLDrawable> shown) {
                const double arrival = CACurrentMediaTime();
                ++state->presented;
                if ([shown respondsToSelector:@selector(presentedTime)]) {
                    const double time = shown.presentedTime;
                    if (time == 0.0) ++state->zero;
                    else if (!std::isfinite(time) || time < 0) ++state->unavailable;
                    state->record("presented",seq,arrival,time,double(shown.drawableID),std::isfinite(time) && time>0 ? 1:0,content,drawableID);
                } else { ++state->unavailable; state->record("presented",seq,arrival,0,0,0,content,drawableID); }
            }];
        } else { ++state->unavailable; state->record("presented_unavailable",seq,now,0,0,0,content,drawableID); }
        const double beforeCommit = CACurrentMediaTime();
        [command commit];
        ++state->submitted;
        state->record("commit",seq,CACurrentMediaTime(),(CACurrentMediaTime()-beforeCommit)*1e6,(beforeCommit-now)*1e6,0,content,drawableID);
        // CAMetalDisplayLink explicitly requires commit, then ordinary present().
        // Do not use timed present variants or commandBuffer.presentDrawable here.
        const double beforePresent = CACurrentMediaTime();
        [drawable present];
        state->record("present_call",seq,CACurrentMediaTime(),(CACurrentMediaTime()-beforePresent)*1e6,0,0,content,drawableID);
        // The autorelease pool and local strong references end with this callback.
    }
}
- (void)stop {
    NSCAssert([NSThread isMainThread],@"Stop/lifecycle is owned by main thread");
    if (_closed) return;
    _closed = YES;
    stats->stop = true;
    if (_worker.joinable()) _worker.join(); // Worker never dispatches synchronously to main.
    const long drain = dispatch_group_wait(_gpuWork,dispatch_time(DISPATCH_TIME_NOW,2*NSEC_PER_SEC));
    std::fprintf(stderr,"native_stop host_s=%.9f gpu_drain=%s inflight=%u\n",CACurrentMediaTime(),drain==0?"complete":"timeout",stats->inflight.load());
    // If a GPU drain times out, Metal callbacks retain only shared state/semaphore/group.
    // Never free their storage prematurely or access AppKit from those callbacks.
    report(stats);
}
- (BOOL)isOpen { NSCAssert([NSThread isMainThread],@"Main thread only"); return !_closed; }
- (void)windowWillClose:(NSNotification *)notification { [self stop]; }
- (void)dealloc { NSCAssert(!_worker.joinable(),@"Explicit stop must precede deallocation"); }
@end

extern "C" void *rastera_native_create(void **statsOut, bool disableAnimations) {
    NSCAssert([NSThread isMainThread],@"Main thread only");
    *statsOut = nullptr;
    if (@available(macOS 14.0,*)) {
        RasteraNativePresenter *presenter = [RasteraNativePresenter new];
        if (![presenter start:disableAnimations]) return nullptr;
        *statsOut = new StatsHandle(presenter->stats);
        return (__bridge_retained void *)presenter;
    }
    std::fprintf(stderr,"native_error requires_macos=14.0\n");
    return nullptr;
}
extern "C" void rastera_native_report(void *handle) { report(*static_cast<StatsHandle *>(handle)); }
extern "C" void rastera_native_stats_destroy(void *handle) { delete static_cast<StatsHandle *>(handle); }
extern "C" bool rastera_native_is_open(void *handle) {
    if (@available(macOS 14.0,*)) return [(__bridge RasteraNativePresenter *)handle isOpen];
    return false;
}
extern "C" void rastera_native_destroy(void *handle) {
    if (@available(macOS 14.0,*)) {
        RasteraNativePresenter *presenter = (__bridge_transfer RasteraNativePresenter *)handle;
        [presenter stop];
        presenter->window.delegate = nil;
        [presenter->window close];
        presenter->window.contentView = nil;
    }
}

// Serialize Rust and native diagnostic text through the same FILE lock. Rust's
// formatted stderr writes otherwise interleave with C stdio at initialization.
extern "C" void rastera_native_log(const char *line) { std::fputs(line,stderr); }
