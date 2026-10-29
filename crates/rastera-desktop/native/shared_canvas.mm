// Native consumers of completed rastera-core frames. AppKit stays on main.
#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <QuartzCore/QuartzCore.h>
#include <atomic>
#include <algorithm>
#include <cstdio>
#include <cmath>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>
#include <cstring>
struct SharedFrame { void *texture; void *lease; uint64_t frame; };
extern "C" bool rastera_shared_acquire(void *, SharedFrame *);
extern "C" void rastera_shared_release(void *);
extern "C" void rastera_shared_fit(uint32_t,uint32_t,float,float,float *);
namespace {
struct Event { const char *kind; uint64_t frame,drawable,seq; double host,time; };
struct Stats {
    const char *role;
    std::mutex mutex;
    std::deque<Event> events;
    std::atomic<uint64_t> ticks{0},submitted{0},completed{0},presented{0},zero{0},errors{0},lost{0},busy{0};
    std::atomic<unsigned> inflight{0},maxInflight{0};
    std::atomic<bool> stop{false};
    explicit Stats(const char *r):role(r) {}
    void record(const char *kind,uint64_t frame,uint64_t drawable,double time=0,uint64_t seq=0) {
        const double now=CACurrentMediaTime();
        std::lock_guard<std::mutex> guard(mutex);
        if(events.size()==4096) { events.pop_front(); ++lost; }
        events.push_back({kind,frame,drawable,seq,now,time});
    }
};
std::mutex registryMutex;
std::vector<std::weak_ptr<Stats>> registry;
void report(const std::shared_ptr<Stats> &s) {
    std::deque<Event> rows;
    { std::lock_guard<std::mutex> guard(s->mutex); rows.swap(s->events); }
    for(const auto &e:rows) std::fprintf(stderr,"shared_event role=%s kind=%s frame=%llu drawable=%llu seq=%llu callback_host_s=%.9f gpu_or_presented_host_s=%.9f\n",s->role,e.kind,(unsigned long long)e.frame,(unsigned long long)e.drawable,(unsigned long long)e.seq,e.host,e.time);
    std::fprintf(stderr,"shared_interval role=%s host_s=%.9f ticks=%llu submitted=%llu completed=%llu presented_callbacks=%llu zero=%llu errors=%llu lost=%llu busy=%llu inflight=%u max_inflight=%u\n",s->role,CACurrentMediaTime(),(unsigned long long)s->ticks.load(),(unsigned long long)s->submitted.load(),(unsigned long long)s->completed.load(),(unsigned long long)s->presented.load(),(unsigned long long)s->zero.load(),(unsigned long long)s->errors.load(),(unsigned long long)s->lost.load(),(unsigned long long)s->busy.load(),s->inflight.load(),s->maxInflight.load());
}
}
// Operator-only surface state. Never sampled by the canvas producer or clean windows.
struct EditorPointer { uint32_t kind; float x,y,width,height,scale; };
@interface EditorSelectionView : NSView {
@public float selection[4]; uint32_t flags; CGSize pixels;
}
@end
@implementation EditorSelectionView
- (BOOL)isFlipped { return YES; }
- (NSView *)hitTest:(NSPoint)point { return nil; }
- (void)drawRect:(NSRect)dirty {
    NSRectFillUsingOperation(dirty,NSCompositingOperationClear);
    if(!flags || pixels.width<=0 || pixels.height<=0) return;
    float fit[4]; rastera_shared_fit(3840,1080,float(pixels.width),float(pixels.height),fit);
    const float sx=self.bounds.size.width/pixels.width, sy=self.bounds.size.height/pixels.height;
    NSRect r=NSMakeRect((fit[0]+selection[0]/3840*fit[2])*sx,(fit[1]+selection[1]/1080*fit[3])*sy,selection[2]/3840*fit[2]*sx,selection[3]/1080*fit[3]*sy);
    NSColor *stroke=(flags&2)?NSColor.grayColor:NSColor.cyanColor; [stroke setStroke];
    NSBezierPath *outline=[NSBezierPath bezierPathWithRect:r]; outline.lineWidth=1.5; [outline stroke];
    if(!(flags&2)) { [NSColor.whiteColor setFill];
        for(int i=0;i<4;++i) { NSPoint p=NSMakePoint(i%2?NSMaxX(r):NSMinX(r),i/2?NSMaxY(r):NSMinY(r));
            NSBezierPath *handle=[NSBezierPath bezierPathWithRect:NSMakeRect(p.x-4,p.y-4,8,8)]; [handle fill];[handle stroke]; }
    }
}
@end
@interface SharedCanvasView : NSView {
@public BOOL editorEnabled; std::deque<EditorPointer> pointerEvents; EditorSelectionView *selectionView;
}
- (void)resizeDrawable;
@end
@implementation SharedCanvasView
- (BOOL)acceptsFirstMouse:(NSEvent *)event { return editorEnabled?YES:[super acceptsFirstMouse:event]; }
- (void)recordPointer:(NSEvent *)event kind:(uint32_t)kind {
    NSPoint p=[self convertPoint:event.locationInWindow fromView:nil];
    CGSize size=((CAMetalLayer *)self.layer).drawableSize;
    float sx=self.bounds.size.width>0?size.width/self.bounds.size.width:1;
    float sy=self.bounds.size.height>0?size.height/self.bounds.size.height:1;
    EditorPointer value={kind,float(p.x*sx),float((self.bounds.size.height-p.y)*sy),float(size.width),float(size.height),sx};
    if(kind==2&&!pointerEvents.empty()&&pointerEvents.back().kind==2) pointerEvents.back()=value;
    else { if(pointerEvents.size()>=64) {pointerEvents.clear();pointerEvents.push_back({4,0,0,0,0,1});} pointerEvents.push_back(value); }
}
- (void)mouseDown:(NSEvent *)event { if(editorEnabled) { [self.window makeFirstResponder:self.superview]; [self recordPointer:event kind:1]; } else [super mouseDown:event]; }
- (void)mouseDragged:(NSEvent *)event { if(editorEnabled) [self recordPointer:event kind:2]; else [super mouseDragged:event]; }
- (void)mouseUp:(NSEvent *)event { if(editorEnabled) [self recordPointer:event kind:3]; else [super mouseUp:event]; }
- (CALayer *)makeBackingLayer { return [CAMetalLayer layer]; }
- (BOOL)wantsUpdateLayer { return YES; }
- (void)resizeDrawable {
    NSCAssert(NSThread.isMainThread,@"View operations on main");
    NSSize size=[self convertRectToBacking:self.bounds].size;
    if(size.width<=0||size.height<=0) return;
    [CATransaction begin]; [CATransaction setDisableActions:YES];
    CAMetalLayer *layer=(CAMetalLayer *)self.layer;
    layer.contentsScale=self.window.backingScaleFactor?:1;
    layer.drawableSize=CGSizeMake(size.width,size.height);
    [CATransaction commit];
}
- (void)setFrameSize:(NSSize)size { [super setFrameSize:size]; [self resizeDrawable]; }
- (void)viewDidChangeBackingProperties { [super viewDidChangeBackingProperties]; [self resizeDrawable]; }
- (void)viewDidMoveToWindow { [super viewDidMoveToWindow]; [self resizeDrawable]; }
@end
API_AVAILABLE(macos(14.0))
@interface SharedCanvasConsumer : NSObject <NSWindowDelegate,CAMetalDisplayLinkDelegate> {
    NSWindow *_window;
    SharedCanvasView *_view;
    CAMetalLayer *_layer;
    id<MTLDevice> _device;
    id<MTLCommandQueue> _queue;
    id<MTLRenderPipelineState> _pipeline;
    void *_source;
    uint32_t _slice[4];
    BOOL _embedded;
    BOOL _closed;
    std::shared_ptr<Stats> _stats;
    std::thread _worker;
    dispatch_semaphore_t _slots;
    dispatch_group_t _work;
}
- (BOOL)start:(void *)source host:(NSView *)host role:(int)role slice:(const uint32_t *)slice;
- (void)stop;
- (BOOL)isOpen;
- (void)editorRect:(const float *)rect screen:(const float *)screen selection:(const float *)selected flags:(uint32_t)flags;
- (BOOL)pollPointer:(EditorPointer *)out;
- (void)replay:(uint32_t)kind x:(float)x y:(float)y;
- (void)closeOutputForProbe;
@end
@implementation SharedCanvasConsumer
- (BOOL)start:(void *)source host:(NSView *)host role:(int)role slice:(const uint32_t *)slice {
    NSCAssert(NSThread.isMainThread,@"Lifecycle on main");
    _source=source; _embedded=host!=nil; std::memcpy(_slice,slice,sizeof(_slice));
    _stats=std::make_shared<Stats>(role==0?"preview":role==1?"left":"right");
    _slots=dispatch_semaphore_create(2); _work=dispatch_group_create();
    SharedFrame first{};
    if(!rastera_shared_acquire(source,&first)) return NO;
    id<MTLTexture> texture=(__bridge id<MTLTexture>)first.texture;
    _device=texture.device; // Use the exact producer MTLDevice, not an assumed default GPU.
    const bool compatible=texture.width==3840 && texture.height==1080 && texture.pixelFormat==MTLPixelFormatRGBA8Unorm;
    std::fprintf(stderr,"shared_device role=%s registry_id=%llu exact_producer_device=1 dimensions=%lux%lu compatible=%d reduce_motion=%d main_thread=1\n",_stats->role,(unsigned long long)_device.registryID,(unsigned long)texture.width,(unsigned long)texture.height,compatible,NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion);
    rastera_shared_release(first.lease);
    if(!compatible) return NO;
    _queue=[_device newCommandQueue];
    NSString *shader=[NSString stringWithUTF8String:R"MSL(
#include <metal_stdlib>
using namespace metal;
struct Mapping { float4 slice; float4 fit; };
vertex float4 vertex_main(uint i [[vertex_id]]) {
    const float2 p[3]={float2(-1,-1),float2(3,-1),float2(-1,3)};
    return float4(p[i],0,1);
}
fragment float4 fragment_main(float4 p [[position]],constant Mapping &m [[buffer(0)]],texture2d<float> canvas [[texture(0)]]) {
    float2 uv=(p.xy-m.fit.xy)/m.fit.zw;
    if(any(uv<0.0)||any(uv>=1.0)) return float4(0,0,0,1);
    uint2 pixel=uint2(min(floor(m.slice.xy+uv*m.slice.zw),m.slice.xy+m.slice.zw-1.0));
    return canvas.read(pixel);
}
)MSL"];
    NSError *error=nil;
    id<MTLLibrary> library=[_device newLibraryWithSource:shader options:nil error:&error];
    if(!library) { NSLog(@"Shared shader: %@",error); return NO; }
    MTLRenderPipelineDescriptor *desc=[MTLRenderPipelineDescriptor new];
    desc.vertexFunction=[library newFunctionWithName:@"vertex_main"];
    desc.fragmentFunction=[library newFunctionWithName:@"fragment_main"];
    desc.colorAttachments[0].pixelFormat=MTLPixelFormatBGRA8Unorm;
    _pipeline=[_device newRenderPipelineStateWithDescriptor:desc error:&error];
    if(!_queue||!_pipeline) { NSLog(@"Shared pipeline: %@",error); return NO; }
    if(_embedded) {
        _window=host.window;
        _view=[[SharedCanvasView alloc] initWithFrame:NSMakeRect(0,host.isFlipped?180.0:0.0,host.bounds.size.width,std::max(1.0,host.bounds.size.height-180.0))];
        _view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
        [host addSubview:_view];
    } else {
        _window=[[NSWindow alloc] initWithContentRect:NSMakeRect(role==1?20:520,120,480,270)
            styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable
            backing:NSBackingStoreBuffered defer:NO];
        _window.title=role==1?@"Rastera — Native clean output — Left":@"Rastera — Native clean output — Right";
        _window.releasedWhenClosed=NO; _window.delegate=self;
        _view=[[SharedCanvasView alloc] initWithFrame:NSMakeRect(0,0,480,270)];
        _window.contentView=_view;
    }
    _view.wantsLayer=YES; _layer=(CAMetalLayer *)_view.layer;
    _layer.device=_device; _layer.pixelFormat=MTLPixelFormatBGRA8Unorm;
    _layer.framebufferOnly=YES; _layer.maximumDrawableCount=3;
    _layer.presentsWithTransaction=NO; _layer.displaySyncEnabled=YES;
    _layer.allowsNextDrawableTimeout=YES;
    if(!_embedded) {
        BOOL replay=NO;for(NSString *arg in NSProcessInfo.processInfo.arguments)if([arg hasPrefix:@"--editor-probe="])replay=YES;
        if(replay){_window.title=[_window.title stringByAppendingString:@" (automated probe)"];[_window orderFront:nil];}else [_window makeKeyAndOrderFront:nil];
    }
    [_view resizeDrawable];
    { std::lock_guard<std::mutex> guard(registryMutex); registry.push_back(_stats); }
    std::fprintf(stderr,"shared_open role=%s x=%u y=%u width=%u height=%u animation_readback=%ld\n",_stats->role,_slice[0],_slice[1],_slice[2],_slice[3],(long)_window.animationBehavior);
    SharedCanvasConsumer *owner=self;
    _worker=std::thread([owner] {
        @autoreleasepool {
            NSCAssert(!NSThread.isMainThread,@"Renderer independent of UI");
            CAMetalDisplayLink *link=[[CAMetalDisplayLink alloc] initWithMetalLayer:owner->_layer];
            link.delegate=owner; link.preferredFrameLatency=2;
            link.preferredFrameRateRange=CAFrameRateRangeMake(60,60,60);
            [link addToRunLoop:NSRunLoop.currentRunLoop forMode:NSDefaultRunLoopMode];
            while(!owner->_stats->stop.load()) { @autoreleasepool { CFRunLoopRunInMode(kCFRunLoopDefaultMode,0.05,false); } }
            [link invalidate]; link.delegate=nil;
        }
    });
    return YES;
}
- (void)metalDisplayLink:(CAMetalDisplayLink *)link needsUpdate:(CAMetalDisplayLinkUpdate *)update {
    @autoreleasepool {
        NSCAssert(!NSThread.isMainThread,@"GPU reads on worker");
        auto stats=_stats;
        if(stats->stop.load()) return;
        const uint64_t seq=++stats->ticks;
        if(dispatch_semaphore_wait(_slots,DISPATCH_TIME_NOW)!=0) { ++stats->busy; return; }
        SharedFrame frame{};
        if(!rastera_shared_acquire(_source,&frame)) { dispatch_semaphore_signal(_slots); return; }
        id<MTLTexture> source=(__bridge id<MTLTexture>)frame.texture;
        id<CAMetalDrawable> drawable=update.drawable;
        if(source.device!=_device || source.device.registryID!=_device.registryID || !drawable ||
           _slice[0]+_slice[2]>source.width || _slice[1]+_slice[3]>source.height) {
            ++stats->errors; stats->stop=true; rastera_shared_release(frame.lease); dispatch_semaphore_signal(_slots); return;
        }
        const uint64_t drawableID=drawable.drawableID;
        stats->record("schedule",frame.frame,drawableID,0,seq);
        id<MTLCommandBuffer> command=[_queue commandBuffer];
        MTLRenderPassDescriptor *pass=[MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture=drawable.texture;
        pass.colorAttachments[0].loadAction=MTLLoadActionClear; pass.colorAttachments[0].storeAction=MTLStoreActionStore;
        pass.colorAttachments[0].clearColor=MTLClearColorMake(0,0,0,1);
        id<MTLRenderCommandEncoder> encoder=[command renderCommandEncoderWithDescriptor:pass];
        if(!encoder) { ++stats->errors; rastera_shared_release(frame.lease); dispatch_semaphore_signal(_slots); return; }
        struct Mapping { float slice[4],fit[4]; } mapping;
        for(int i=0;i<4;++i) mapping.slice[i]=float(_slice[i]);
        rastera_shared_fit(_slice[2],_slice[3],float(drawable.texture.width),float(drawable.texture.height),mapping.fit);
        [encoder setRenderPipelineState:_pipeline];
        [encoder setFragmentBytes:&mapping length:sizeof(mapping) atIndex:0];
        [encoder setFragmentTexture:source atIndex:0];
        [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3]; [encoder endEncoding];
        const unsigned active=++stats->inflight;
        unsigned max=stats->maxInflight.load(); while(active>max&&!stats->maxInflight.compare_exchange_weak(max,active)) {}
        dispatch_semaphore_t slots=_slots; dispatch_group_t work=_work;
        dispatch_group_enter(work);
        // The lease is released ONLY after the last command that reads this texture finishes.
        [command addCompletedHandler:^(id<MTLCommandBuffer> finished) {
            stats->record("complete",frame.frame,drawableID,finished.GPUEndTime,seq);
            if(finished.status==MTLCommandBufferStatusError) { ++stats->errors; stats->stop=true; }
            rastera_shared_release(frame.lease);
            ++stats->completed; --stats->inflight; dispatch_semaphore_signal(slots); dispatch_group_leave(work);
        }];
        [drawable addPresentedHandler:^(id<MTLDrawable> shown) {
            const double time=shown.presentedTime;
            if(time==0) ++stats->zero;
            else if(!std::isfinite(time)||time<0) ++stats->errors;
            ++stats->presented; stats->record("presented",frame.frame,drawableID,time,seq);
        }];
        [command commit]; ++stats->submitted; stats->record("commit",frame.frame,drawableID,0,seq);
        [drawable present];
    }
}
- (void)editorRect:(const float *)rect screen:(const float *)screen selection:(const float *)selected flags:(uint32_t)flags {
    NSCAssert(NSThread.isMainThread,@"Editor layout/overlay on main");
    if(!_embedded||screen[0]<=0||screen[1]<=0) return;
    NSView *host=_view.superview;
    float sx=host.bounds.size.width/screen[0], sy=host.bounds.size.height/screen[1];
    NSRect frame=NSMakeRect(rect[0]*sx,host.isFlipped?rect[1]*sy:host.bounds.size.height-(rect[1]+rect[3])*sy,rect[2]*sx,rect[3]*sy);
    _view.autoresizingMask=NSViewNotSizable;
    _view->editorEnabled=YES;
    if(!NSEqualRects(frame,_view.frame)) _view.frame=frame;
    if(!_view->selectionView) {
        _view->selectionView=[[EditorSelectionView alloc] initWithFrame:_view.bounds];
        _view->selectionView.wantsLayer=YES;
        _view->selectionView.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
        [_view addSubview:_view->selectionView];
    }
    EditorSelectionView *overlay=_view->selectionView;
    const CGSize size=_layer.drawableSize;
    if(overlay->flags!=flags || std::memcmp(overlay->selection,selected,4*sizeof(float)) || !CGSizeEqualToSize(overlay->pixels,size)) {
        overlay->flags=flags;overlay->pixels=size;std::memcpy(overlay->selection,selected,4*sizeof(float));overlay.needsDisplay=YES;
    }
}
- (void)closeOutputForProbe { NSCAssert(NSThread.isMainThread,@"Close on main");if(!_embedded)[_window performClose:nil]; }
- (void)replay:(uint32_t)kind x:(float)x y:(float)y {
    if(!_embedded||!_view->editorEnabled)return;
    float fit[4];const CGSize size=_layer.drawableSize;rastera_shared_fit(3840,1080,size.width,size.height,fit);
    NSPoint local=NSMakePoint((fit[0]+x/3840*fit[2])*_view.bounds.size.width/size.width,_view.bounds.size.height-(fit[1]+y/1080*fit[3])*_view.bounds.size.height/size.height);
    NSPoint position=[_view convertPoint:local toView:nil];
    NSEventType type=kind==1?NSEventTypeLeftMouseDown:kind==2?NSEventTypeLeftMouseDragged:NSEventTypeLeftMouseUp;
    NSEvent *event=[NSEvent mouseEventWithType:type location:position modifierFlags:0 timestamp:CACurrentMediaTime() windowNumber:_window.windowNumber context:nil eventNumber:0 clickCount:1 pressure:0];
    if(kind==1)[_view mouseDown:event];else if(kind==2)[_view mouseDragged:event];else [_view mouseUp:event];
}
- (BOOL)pollPointer:(EditorPointer *)out {
    NSCAssert(NSThread.isMainThread,@"Editor input on main");
    if(_view->pointerEvents.empty()) return NO;
    *out=_view->pointerEvents.front();_view->pointerEvents.pop_front();return YES;
}
- (BOOL)isOpen { NSCAssert(NSThread.isMainThread,@"Main only"); return !_closed && !_stats->stop.load(); }
- (void)stop {
    NSCAssert(NSThread.isMainThread,@"Main lifecycle");
    if(_closed) return; _closed=YES;
    _stats->stop=true;
    if(_worker.joinable()) _worker.join();
    const long result=dispatch_group_wait(_work,dispatch_time(DISPATCH_TIME_NOW,2*NSEC_PER_SEC));
    std::fprintf(stderr,"shared_stop role=%s host_s=%.9f gpu_drain=%s inflight=%u\n",_stats->role,CACurrentMediaTime(),result==0?"complete":"timeout",_stats->inflight.load());
    report(_stats);
}
- (void)windowWillClose:(NSNotification *)note { [self stop]; }
- (void)detach { [self stop]; if(!_embedded) { _window.delegate=nil; [_window close]; } [_view removeFromSuperview]; }
- (void)dealloc { NSCAssert(!_worker.joinable(),@"Must stop before release"); }
@end
extern "C" void *rastera_shared_create(void *source,void *host,int role,const uint32_t *slice) {
    if(@available(macOS 14.0,*)) {
        SharedCanvasConsumer *consumer=[SharedCanvasConsumer new];
        if(![consumer start:source host:(__bridge NSView *)host role:role slice:slice]) return nullptr;
        return (__bridge_retained void *)consumer;
    }
    return nullptr;
}
extern "C" void rastera_shared_destroy(void *handle) {
    if(@available(macOS 14.0,*)) { SharedCanvasConsumer *consumer=(__bridge_transfer SharedCanvasConsumer *)handle; [consumer detach]; }
}
extern "C" bool rastera_shared_is_open(void *handle) {
    if(@available(macOS 14.0,*)) return [(__bridge SharedCanvasConsumer *)handle isOpen];
    return false;
}
extern "C" void rastera_shared_report() {
    std::lock_guard<std::mutex> guard(registryMutex);
    for(auto i=registry.begin();i!=registry.end();) { if(auto s=i->lock()) { report(s); ++i; } else i=registry.erase(i); }
}
extern "C" void rastera_shared_log(const char *line) { std::fputs(line,stderr); }

extern "C" double rastera_shared_host_time() { return CACurrentMediaTime(); }

extern "C" void rastera_editor_layout(void *handle,const float *rect,const float *screen,const float *selection,uint32_t flags) {
    if(@available(macOS 14.0,*)) [(__bridge SharedCanvasConsumer *)handle editorRect:rect screen:screen selection:selection flags:flags];
}
extern "C" bool rastera_editor_pointer(void *handle,EditorPointer *out) {
    if(@available(macOS 14.0,*)) return [(__bridge SharedCanvasConsumer *)handle pollPointer:out];
    return false;
}
// Native panels are UI operations; all subsequent reading/decoding/writing is on a worker.
extern "C" char *rastera_editor_dialog(int kind) {
    NSCAssert(NSThread.isMainThread,@"Native file panels on main");
    if(kind==2) {
        NSSavePanel *panel=[NSSavePanel savePanel];panel.allowedContentTypes=@[UTTypeJSON];panel.nameFieldStringValue=@"Untitled.rastera.json";
        if([panel runModal]!=NSModalResponseOK) return nullptr;
        return strdup(panel.URL.path.UTF8String);
    }
    NSOpenPanel *panel=[NSOpenPanel openPanel];panel.canChooseDirectories=NO;panel.allowsMultipleSelection=NO;
    panel.allowedContentTypes=kind==0?@[UTTypePNG,UTTypeJPEG]:@[UTTypeJSON];
    if([panel runModal]!=NSModalResponseOK) return nullptr;
    return strdup(panel.URL.path.UTF8String);
}
extern "C" void rastera_editor_free_string(char *value) { free(value); }
// Opt-in diagnostic replay only: dispatch native view events on main without
// activating the app, posting system input or touching the user's cursor.
extern "C" void rastera_editor_replay(void *handle,uint32_t kind,float x,float y) {
    NSCAssert(NSThread.isMainThread,@"Replay on main");
    if(@available(macOS 14.0,*)) {
        SharedCanvasConsumer *consumer=(__bridge SharedCanvasConsumer *)handle;
        [consumer replay:kind x:x y:y];
    }
}

extern "C" void rastera_editor_close_output(void *handle) {
    if(@available(macOS 14.0,*)) [(__bridge SharedCanvasConsumer *)handle closeOutputForProbe];
}
