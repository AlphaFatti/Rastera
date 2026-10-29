// Native consumers of completed rastera-core frames. AppKit stays on main.
#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <CoreText/CoreText.h>
#import <objc/runtime.h>
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
#include <string>
#include <cstddef>
struct FrameViewport {uint64_t id;uint32_t width,height;float matrix[8];};
struct SharedFrame { void *texture; void *lease; uint64_t frame; uint32_t width,height; const FrameViewport *viewports;uint32_t viewportCount,reserved; };
static_assert(sizeof(FrameViewport)==48&&offsetof(FrameViewport,matrix)==16,"Viewport ABI must match Rust");
static_assert(sizeof(SharedFrame)==48&&offsetof(SharedFrame,viewports)==32,"Shared frame ABI must match Rust");
static const FrameViewport *viewportFor(const SharedFrame &frame,uint64_t id){
    if(frame.viewportCount>8||(!frame.viewports&&frame.viewportCount))return nullptr;
    for(uint32_t i=0;i<frame.viewportCount;++i)if(frame.viewports[i].id==id)return &frame.viewports[i];return nullptr;
}
extern "C" bool rastera_shared_acquire(void *, SharedFrame *);
extern "C" void rastera_shared_release(void *);
extern "C" void rastera_shared_fit(uint32_t,uint32_t,float,float,float *);
extern "C" void rastera_editor_fit(uint32_t,uint32_t,float,float,float,float *);
extern "C" void rastera_theme_workspace(float *);
namespace {
struct Event { const char *kind; uint64_t frame,drawable,seq; double host,time; };
std::atomic<uint64_t> nextConsumerInstance{1};
struct Stats {
    std::string role;uint64_t id,instance;
    std::mutex mutex;
    std::deque<Event> events;
    std::atomic<uint64_t> ticks{0},submitted{0},completed{0},presented{0},zero{0},errors{0},lost{0},busy{0};
    std::atomic<unsigned> inflight{0},maxInflight{0};
    std::atomic<bool> stop{false};
    explicit Stats(uint64_t n):role(n==0?"preview":n==1?"left":n==2?"right":("viewport-"+std::to_string(n))),id(n),instance(nextConsumerInstance.fetch_add(1)) {}
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
    for(const auto &e:rows) std::fprintf(stderr,"shared_event role=%s viewport_id=%llu instance_id=%llu kind=%s frame=%llu drawable=%llu seq=%llu callback_host_s=%.9f gpu_or_presented_host_s=%.9f\n",s->role.c_str(),(unsigned long long)s->id,(unsigned long long)s->instance,e.kind,(unsigned long long)e.frame,(unsigned long long)e.drawable,(unsigned long long)e.seq,e.host,e.time);
    std::fprintf(stderr,"shared_interval role=%s viewport_id=%llu instance_id=%llu host_s=%.9f ticks=%llu submitted=%llu completed=%llu presented_callbacks=%llu zero=%llu errors=%llu lost=%llu busy=%llu inflight=%u max_inflight=%u\n",s->role.c_str(),(unsigned long long)s->id,(unsigned long long)s->instance,CACurrentMediaTime(),(unsigned long long)s->ticks.load(),(unsigned long long)s->submitted.load(),(unsigned long long)s->completed.load(),(unsigned long long)s->presented.load(),(unsigned long long)s->zero.load(),(unsigned long long)s->errors.load(),(unsigned long long)s->lost.load(),(unsigned long long)s->busy.load(),s->inflight.load(),s->maxInflight.load());
}
}
// Operator-only surface state. Never sampled by the canvas producer or clean windows.
struct EditorPointer { uint32_t kind; float x,y,width,height,scale; uint64_t modifiers; };
static_assert(sizeof(EditorPointer)==32,"Pointer ABI must match Rust");
static NSFont *EditorLabelFont(){return [NSFont systemFontOfSize:12 weight:NSFontWeightSemibold];}
struct OverlayLine {float x1,y1,x2,y2,r,g,b,a,width;uint32_t kind;};
struct OverlayLabel {float x,y;const char *text;float color[4];};
@interface EditorSelectionView : NSView {
@public float selection[4]; uint32_t flags; CGSize pixels;uint32_t canvas[2];std::vector<OverlayLine> lines;NSArray *labels;
}
@end
@implementation EditorSelectionView
- (BOOL)isFlipped { return YES; }
- (NSView *)hitTest:(NSPoint)point { return nil; }
- (void)drawRect:(NSRect)dirty {
    [NSGraphicsContext saveGraphicsState];
    [NSBezierPath clipRect:self.bounds];
    NSRectFillUsingOperation(dirty,NSCompositingOperationClear);
    if(pixels.width<=0||canvas[0]==0){[NSGraphicsContext restoreGraphicsState];return;}
    float fit[4];const float sx=self.bounds.size.width/pixels.width,sy=self.bounds.size.height/pixels.height;
    rastera_editor_fit(canvas[0],canvas[1],pixels.width,pixels.height,20./sx,fit);
    auto point=[&](float x,float y){return NSMakePoint((fit[0]+x/canvas[0]*fit[2])*sx,(fit[1]+y/canvas[1]*fit[3])*sy);};
    for(const auto &l:lines){NSColor *color=[NSColor colorWithSRGBRed:l.r green:l.g blue:l.b alpha:l.a];[color setStroke];[color setFill];NSPoint a=point(l.x1,l.y1),b=point(l.x2,l.y2);NSBezierPath *path;
        if(l.kind==0){path=[NSBezierPath bezierPath];[path moveToPoint:a];[path lineToPoint:b];}
        else {NSRect r=NSMakeRect(a.x-4,a.y-4,8,8);path=l.kind==1?[NSBezierPath bezierPathWithRect:r]:[NSBezierPath bezierPathWithOvalInRect:r];[path fill];}
        path.lineWidth=l.width;[path stroke];
    }
    float bg[4];rastera_theme_workspace(bg);
    for(NSDictionary *label in labels){NSPoint p=point([label[@"x"] floatValue],[label[@"y"] floatValue]);[label[@"text"] drawAtPoint:NSMakePoint(p.x+6,p.y+6) withAttributes:@{NSFontAttributeName:EditorLabelFont(),NSForegroundColorAttributeName:label[@"color"],NSBackgroundColorAttributeName:[NSColor colorWithSRGBRed:bg[0] green:bg[1] blue:bg[2] alpha:0.9]}];}
    [NSGraphicsContext restoreGraphicsState];
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
    EditorPointer value={kind,float(p.x*sx),float((self.bounds.size.height-p.y)*sy),float(size.width),float(size.height),sx,uint64_t(event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask)};
    if(kind==2&&!pointerEvents.empty()&&pointerEvents.back().kind==2) pointerEvents.back()=value;
    else { if(pointerEvents.size()>=64) {pointerEvents.clear();pointerEvents.push_back({4,0,0,0,0,1,0});} pointerEvents.push_back(value); }
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
    BOOL _embedded;
    uint64_t _viewportID;
    uint32_t _sizedLogical[2];
    uint32_t _lastWidth,_lastHeight,_lastLogical[2]; float _lastMaps[8];
    std::atomic<float> _margin;
    BOOL _closed;
    std::shared_ptr<Stats> _stats;
    std::thread _worker;
    dispatch_semaphore_t _slots;
    dispatch_group_t _work;
}
- (BOOL)start:(void *)source host:(NSView *)host viewport:(uint64_t)viewportID;
- (void)stop;
- (BOOL)isOpen;
- (uint32_t)inflightForProbe;
- (uint64_t)instanceForProbe;
- (BOOL)pendingReadForProbe;
- (void)syncWindowSize;
- (void)editorRect:(const float *)rect screen:(const float *)screen selection:(const float *)selected flags:(uint32_t)flags;
- (BOOL)pollPointer:(EditorPointer *)out;
- (void)replay:(uint32_t)kind x:(float)x y:(float)y modifiers:(uint64_t)modifiers;
- (void)closeOutputForProbe;
- (void)overlay:(const uint32_t *)canvas lines:(const OverlayLine *)lines count:(uint32_t)count labels:(const OverlayLabel *)labels labelCount:(uint32_t)labelCount;
- (int)settingsMenu;
- (void)metrics:(float *)out;
- (void)outputTitle:(NSString *)name;
@end
@implementation SharedCanvasConsumer
// AppKit-only sizing; acquire immutable completed metadata, never wait for GPU work.
- (void)syncWindowSize {
    NSCAssert(NSThread.isMainThread,@"Window sizing on main");
    if(_embedded||_closed)return;
    SharedFrame frame{};
    if(!rastera_shared_acquire(_source,&frame))return;
    const FrameViewport *v=viewportFor(frame,_viewportID);
    uint32_t w=v?v->width:0,h=v?v->height:0;
    rastera_shared_release(frame.lease);
    if(!w||!h||(_sizedLogical[0]==w&&_sizedLogical[1]==h))return;
    _sizedLogical[0]=w;_sizedLogical[1]=h;
    // Use AppKit's backing conversion rather than assuming a Retina scale factor.
    NSRect points=[_window convertRectFromBacking:NSMakeRect(0,0,w,h)];
    [_window setContentSize:points.size];
    [_view resizeDrawable];
    NSRect backing=[_view convertRectToBacking:_view.bounds];
    std::fprintf(stderr,"shared_window_size viewport_id=%llu logical_w=%u logical_h=%u backing_w=%.0f backing_h=%.0f points_w=%.3f points_h=%.3f exact=%d main_thread=1\n",(unsigned long long)_viewportID,w,h,backing.size.width,backing.size.height,_view.bounds.size.width,_view.bounds.size.height,int(std::abs(backing.size.width-w)<0.5&&std::abs(backing.size.height-h)<0.5));
}
- (uint64_t)instanceForProbe {return _stats->instance;}
- (uint32_t)inflightForProbe {return _stats->inflight.load();}
// Diagnostic-only actual GPU reader. The normal render/present callback is unchanged.
// A shared event keeps this single private-buffer blit pending for 50 ms, so removal
// exercises the existing completion-held lease and shutdown group with work in flight.
- (BOOL)pendingReadForProbe {
    NSCAssert(NSThread.isMainThread,@"Diagnostic submission on main");
    BOOL enabled=NO;for(NSString *arg in NSProcessInfo.processInfo.arguments)if([arg isEqual:@"--dynamic-probe"])enabled=YES;
    if(!enabled||_embedded||dispatch_semaphore_wait(_slots,DISPATCH_TIME_NOW)!=0)return NO;
    SharedFrame frame{};
    if(!rastera_shared_acquire(_source,&frame)){dispatch_semaphore_signal(_slots);return NO;}
    id<MTLTexture> source=(__bridge id<MTLTexture>)frame.texture;
    id<MTLSharedEvent> event=[_device newSharedEvent];
    id<MTLBuffer> scratch=[_device newBufferWithLength:256 options:MTLResourceStorageModePrivate];
    id<MTLCommandBuffer> command=[_queue commandBuffer];
    if(!event||!scratch||!command||!viewportFor(frame,_viewportID)||source.device!=_device){rastera_shared_release(frame.lease);dispatch_semaphore_signal(_slots);return NO;}
    [command encodeWaitForEvent:event value:1];
    id<MTLBlitCommandEncoder> blit=[command blitCommandEncoder];
    if(!blit){rastera_shared_release(frame.lease);dispatch_semaphore_signal(_slots);return NO;}
    [blit copyFromTexture:source sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0,0,0) sourceSize:MTLSizeMake(1,1,1) toBuffer:scratch destinationOffset:0 destinationBytesPerRow:256 destinationBytesPerImage:256];[blit endEncoding];
    auto stats=_stats;dispatch_group_t work=_work;dispatch_semaphore_t slots=_slots;
    unsigned active=++stats->inflight;unsigned maximum=stats->maxInflight.load();while(active>maximum&&!stats->maxInflight.compare_exchange_weak(maximum,active)){}
    ++stats->submitted;dispatch_group_enter(work);
    [command addCompletedHandler:^(id<MTLCommandBuffer> done){
        stats->record("probe_read_complete",frame.frame,0,done.GPUEndTime);
        if(done.status==MTLCommandBufferStatusError)++stats->errors;
        rastera_shared_release(frame.lease);++stats->completed;--stats->inflight;dispatch_semaphore_signal(slots);dispatch_group_leave(work);
    }];
    stats->record("probe_read_commit",frame.frame,0);[command commit];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,50*NSEC_PER_MSEC),dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0),^{event.signaledValue=1;});
    return YES;
}
- (BOOL)start:(void *)source host:(NSView *)host viewport:(uint64_t)viewportID {
    NSCAssert(NSThread.isMainThread,@"Lifecycle on main");
    _margin.store(0);_viewportID=viewportID;_source=source; _embedded=host!=nil;
    _stats=std::make_shared<Stats>(viewportID);
    _slots=dispatch_semaphore_create(2); _work=dispatch_group_create();
    SharedFrame first{};
    if(!rastera_shared_acquire(source,&first)) return NO;
    id<MTLTexture> texture=(__bridge id<MTLTexture>)first.texture;
    _device=texture.device; // Use the exact producer MTLDevice, not an assumed default GPU.
    const bool compatible=texture.width>0 && texture.height>0 && texture.pixelFormat==MTLPixelFormatRGBA8Unorm;
    std::fprintf(stderr,"shared_device role=%s viewport_id=%llu instance_id=%llu registry_id=%llu exact_producer_device=1 dimensions=%lux%lu compatible=%d reduce_motion=%d main_thread=1\n",_stats->role.c_str(),(unsigned long long)_stats->id,(unsigned long long)_stats->instance,(unsigned long long)_device.registryID,(unsigned long)texture.width,(unsigned long)texture.height,compatible,NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion);
    const FrameViewport *initial=viewportFor(first,viewportID);
    uint32_t logicalWidth=viewportID==0?first.width:(initial?initial->width:0),logicalHeight=viewportID==0?first.height:(initial?initial->height:0);
    rastera_shared_release(first.lease);
    if(!logicalWidth||!logicalHeight)return NO;
    if(!compatible) return NO;
    _queue=[_device newCommandQueue];
    NSString *shader=[NSString stringWithUTF8String:R"MSL(
#include <metal_stdlib>
using namespace metal;
struct Mapping { float4 row0;float4 row1;float4 fit;float4 logical;float4 background; };
vertex float4 vertex_main(uint i [[vertex_id]]) {
    const float2 p[3]={float2(-1,-1),float2(3,-1),float2(-1,3)};
    return float4(p[i],0,1);
}
fragment float4 fragment_main(float4 p [[position]],constant Mapping &m [[buffer(0)]],texture2d<float> canvas [[texture(0)]]) {
    float2 uv=(p.xy-m.fit.xy)/m.fit.zw;
    if(any(uv<0.0)||any(uv>=1.0)) return m.background;
    float2 q=uv*m.logical.xy;
    float2 source=float2(dot(m.row0.xy,q)+m.row0.z,dot(m.row1.xy,q)+m.row1.z);
    if(any(source<0.0)||source.x>=canvas.get_width()||source.y>=canvas.get_height())return float4(0,0,0,1);
    return canvas.read(uint2(floor(source)));
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
        _window=[[NSWindow alloc] initWithContentRect:NSMakeRect(20+((viewportID-1)%4)*350,120,480,std::clamp(480.*logicalHeight/logicalWidth,80.,540.))
            styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable
            backing:NSBackingStoreBuffered defer:NO];
        _window.title=[NSString stringWithFormat:@"Rastera — Native clean output — %@",viewportID==1?@"Left":viewportID==2?@"Right":[NSString stringWithFormat:@"Viewport %llu",(unsigned long long)viewportID]];
        _window.releasedWhenClosed=NO; _window.delegate=self;
        _view=[[SharedCanvasView alloc] initWithFrame:NSMakeRect(0,0,_window.contentView.bounds.size.width,_window.contentView.bounds.size.height)];
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
    [self syncWindowSize];
    [_view resizeDrawable];
    { std::lock_guard<std::mutex> guard(registryMutex); registry.push_back(_stats); }
    std::fprintf(stderr,"shared_open role=%s viewport_id=%llu instance_id=%llu width=%u height=%u animation_readback=%ld\n",_stats->role.c_str(),(unsigned long long)_stats->id,(unsigned long long)_stats->instance,logicalWidth,logicalHeight,(long)_window.animationBehavior);
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
        const FrameViewport *selected=viewportFor(frame,_viewportID);
        if(_viewportID!=0&&!selected){stats->record("missing_viewport",frame.frame,0,0,seq);rastera_shared_release(frame.lease);dispatch_semaphore_signal(_slots);return;}
        id<MTLTexture> source=(__bridge id<MTLTexture>)frame.texture;
        id<CAMetalDrawable> drawable=update.drawable;
        if(source.device!=_device || source.device.registryID!=_device.registryID || !drawable ||
           frame.width!=source.width || frame.height!=source.height) {
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
        struct Mapping { float row0[4],row1[4],fit[4],logical[4],background[4]; } mapping{};
        if(_viewportID==0){mapping.row0[0]=1;mapping.row1[1]=1;mapping.logical[0]=frame.width;mapping.logical[1]=frame.height;}
        else{std::memcpy(mapping.row0,selected->matrix,4*sizeof(float));std::memcpy(mapping.row1,selected->matrix+4,4*sizeof(float));mapping.logical[0]=selected->width;mapping.logical[1]=selected->height;}
        if(_embedded&&_margin.load()>0){rastera_editor_fit(frame.width,frame.height,drawable.texture.width,drawable.texture.height,_margin.load(),mapping.fit);rastera_theme_workspace(mapping.background);}
        else rastera_shared_fit(mapping.logical[0],mapping.logical[1],drawable.texture.width,drawable.texture.height,mapping.fit);
        mapping.background[3]=1;
        if(frame.width!=_lastWidth||frame.height!=_lastHeight||std::memcmp(mapping.row0,_lastMaps,4*sizeof(float))||std::memcmp(mapping.row1,_lastMaps+4,4*sizeof(float))||mapping.logical[0]!=_lastLogical[0]||mapping.logical[1]!=_lastLogical[1]){
            _lastWidth=frame.width;_lastHeight=frame.height;std::memcpy(_lastMaps,mapping.row0,4*sizeof(float));std::memcpy(_lastMaps+4,mapping.row1,4*sizeof(float));_lastLogical[0]=mapping.logical[0];_lastLogical[1]=mapping.logical[1];
            fprintf(stderr,"shared_mapping role=%s viewport_id=%llu instance_id=%llu frame=%llu canvas_w=%u canvas_h=%u logical_w=%u logical_h=%u row00=%.9g row01=%.9g row02=%.9g row10=%.9g row11=%.9g row12=%.9g\n",stats->role.c_str(),(unsigned long long)stats->id,(unsigned long long)stats->instance,(unsigned long long)frame.frame,frame.width,frame.height,_lastLogical[0],_lastLogical[1],mapping.row0[0],mapping.row0[1],mapping.row0[2],mapping.row1[0],mapping.row1[1],mapping.row1[2]);
        }
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
    _view.layer.masksToBounds=YES;
    _margin.store(20.*_layer.contentsScale);
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
- (void)overlay:(const uint32_t *)canvas lines:(const OverlayLine *)lines count:(uint32_t)count labels:(const OverlayLabel *)labels labelCount:(uint32_t)labelCount {
    EditorSelectionView *o=_view->selectionView;if(!o)return;
    BOOL changed=o->canvas[0]!=canvas[0]||o->canvas[1]!=canvas[1]||o->lines.size()!=count;
    if(!changed&&count)changed=std::memcmp(o->lines.data(),lines,count*sizeof(OverlayLine))!=0;
    o->canvas[0]=canvas[0];o->canvas[1]=canvas[1];o->lines.assign(lines,lines+count);
    NSMutableArray *list=[NSMutableArray array];for(uint32_t i=0;i<labelCount;++i){const auto &l=labels[i];[list addObject:@{@"x":@(l.x),@"y":@(l.y),@"text":@(l.text),@"color":[NSColor colorWithSRGBRed:l.color[0] green:l.color[1] blue:l.color[2] alpha:l.color[3]]}];}
    if(![o->labels isEqual:list])changed=YES;o->labels=list;if(changed)o.needsDisplay=YES;
}
- (void)outputTitle:(NSString *)name {if(_embedded)return;NSString *title=[NSString stringWithFormat:@"Rastera — Native clean output — %@ [ID %llu]",name,(unsigned long long)_viewportID];for(NSString *arg in NSProcessInfo.processInfo.arguments)if([arg hasPrefix:@"--editor-probe="])title=[title stringByAppendingString:@" (automated probe)"];if(![_window.title isEqual:title])_window.title=title;}
- (void)metrics:(float *)out {out[0]=_layer.drawableSize.width;out[1]=_layer.drawableSize.height;out[2]=_view.bounds.size.width>0?out[0]/_view.bounds.size.width:1;}
- (int)settingsMenu {
    NSMenu *menu=[[NSMenu alloc]initWithTitle:@"Settings"];NSMenuItem *item=[[NSMenuItem alloc]initWithTitle:@"Canvas settings…" action:@selector(canvasSettingsChosen:) keyEquivalent:@""];item.target=self;[menu addItem:item];
    objc_setAssociatedObject(self,@selector(settingsMenu),@0,OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [menu popUpMenuPositioningItem:nil atLocation:[_view convertPoint:_window.mouseLocationOutsideOfEventStream fromView:nil] inView:_view];
    return [objc_getAssociatedObject(self,@selector(settingsMenu)) intValue];
}
- (void)canvasSettingsChosen:(id)sender {objc_setAssociatedObject(self,@selector(settingsMenu),@1,OBJC_ASSOCIATION_RETAIN_NONATOMIC);}
- (void)closeOutputForProbe { NSCAssert(NSThread.isMainThread,@"Close on main");if(!_embedded)[_window performClose:nil]; }
- (void)replay:(uint32_t)kind x:(float)x y:(float)y modifiers:(uint64_t)modifiers {
    if(!_embedded||!_view->editorEnabled)return;
    float fit[4];const CGSize size=_layer.drawableSize;uint32_t *canvas=_view->selectionView->canvas;rastera_editor_fit(canvas[0],canvas[1],size.width,size.height,_margin.load(),fit);
    NSPoint local=NSMakePoint((fit[0]+x/canvas[0]*fit[2])*_view.bounds.size.width/size.width,_view.bounds.size.height-(fit[1]+y/canvas[1]*fit[3])*_view.bounds.size.height/size.height);
    NSPoint position=[_view convertPoint:local toView:nil];
    NSEventType type=kind==1?NSEventTypeLeftMouseDown:kind==2?NSEventTypeLeftMouseDragged:NSEventTypeLeftMouseUp;
    NSEvent *event=[NSEvent mouseEventWithType:type location:position modifierFlags:(NSEventModifierFlags)modifiers timestamp:CACurrentMediaTime() windowNumber:_window.windowNumber context:nil eventNumber:0 clickCount:1 pressure:0];
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
    std::fprintf(stderr,"shared_stop role=%s viewport_id=%llu instance_id=%llu host_s=%.9f gpu_drain=%s inflight=%u\n",_stats->role.c_str(),(unsigned long long)_stats->id,(unsigned long long)_stats->instance,CACurrentMediaTime(),result==0?"complete":"timeout",_stats->inflight.load());
    report(_stats);
}
- (void)windowWillClose:(NSNotification *)note { [self stop]; }
- (void)detach { [self stop]; if(!_embedded) { _window.delegate=nil; [_window close]; } [_view removeFromSuperview]; }
- (void)dealloc { NSCAssert(!_worker.joinable(),@"Must stop before release"); }
@end
extern "C" void *rastera_shared_create(void *source,void *host,uint64_t viewportID) {
    if(@available(macOS 14.0,*)) {
        SharedCanvasConsumer *consumer=[SharedCanvasConsumer new];
        if(![consumer start:source host:(__bridge NSView *)host viewport:viewportID]) return nullptr;
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
extern "C" void rastera_editor_replay(void *handle,uint32_t kind,float x,float y,uint64_t modifiers) {
    NSCAssert(NSThread.isMainThread,@"Replay on main");
    if(@available(macOS 14.0,*)) {
        SharedCanvasConsumer *consumer=(__bridge SharedCanvasConsumer *)handle;
        [consumer replay:kind x:x y:y modifiers:modifiers];
    }
}

extern "C" void rastera_editor_close_output(void *handle) {
    if(@available(macOS 14.0,*)) [(__bridge SharedCanvasConsumer *)handle closeOutputForProbe];
}

extern "C" void rastera_editor_overlay(void *handle,const uint32_t *canvas,const OverlayLine *lines,uint32_t count,const OverlayLabel *labels,uint32_t labelCount){
    NSCAssert(NSThread.isMainThread,@"Overlay on main");
    if(@available(macOS 14.0,*)){SharedCanvasConsumer *c=(__bridge SharedCanvasConsumer *)handle;[c overlay:canvas lines:lines count:count labels:labels labelCount:labelCount];}
}
extern "C" char *rastera_font_catalog(){@autoreleasepool {
    // Core Text discovery is called from a dedicated worker, never AppKit/view work.
    CTFontCollectionRef collection=CTFontCollectionCreateFromAvailableFonts(NULL);
    NSArray *descriptors=CFBridgingRelease(CTFontCollectionCreateMatchingFontDescriptors(collection));CFRelease(collection);
    NSMutableArray *result=[NSMutableArray array];
    for(id descriptor in descriptors){CTFontDescriptorRef d=(__bridge CTFontDescriptorRef)descriptor;
        NSString *family=CFBridgingRelease(CTFontDescriptorCopyAttribute(d,kCTFontFamilyNameAttribute));
        NSString *style=CFBridgingRelease(CTFontDescriptorCopyAttribute(d,kCTFontStyleNameAttribute));
        NSString *name=CFBridgingRelease(CTFontDescriptorCopyAttribute(d,kCTFontNameAttribute));
        NSURL *url=CFBridgingRelease(CTFontDescriptorCopyAttribute(d,kCTFontURLAttribute));
        if(family&&name&&url.path)[result addObject:@{@"family":family,@"style":style?:@"Regular",@"postscript":name,@"path":url.path}];
    }
    NSData *data=[NSJSONSerialization dataWithJSONObject:result options:0 error:nil];return strdup([[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}}
extern "C" int rastera_settings_menu(void *handle){
    NSCAssert(NSThread.isMainThread,@"Menu on main");
    if(@available(macOS 14.0,*)){return [(__bridge SharedCanvasConsumer *)handle settingsMenu];}return 0;
}

extern "C" void rastera_editor_metrics(void *handle,float *out){NSCAssert(NSThread.isMainThread,@"Metrics on main");if(@available(macOS 14.0,*)){[(__bridge SharedCanvasConsumer *)handle metrics:out];}}
extern "C" void rastera_shared_title(void *handle,const char *name){NSCAssert(NSThread.isMainThread,@"Titles on main");if(@available(macOS 14.0,*)){[(__bridge SharedCanvasConsumer *)handle outputTitle:@(name)];}}

extern "C" void rastera_editor_label_size(const char *text,float *out){
    NSCAssert(NSThread.isMainThread,@"Label measurement on main");
    NSSize size=[@(text) sizeWithAttributes:@{NSFontAttributeName:EditorLabelFont()}];out[0]=size.width;out[1]=size.height;
}

extern "C" uint32_t rastera_shared_inflight(void *handle){
    if(@available(macOS 14.0,*)){return [(__bridge SharedCanvasConsumer *)handle inflightForProbe];}return 0;
}

extern "C" uint64_t rastera_shared_instance(void *handle){if(@available(macOS 14.0,*)){return [(__bridge SharedCanvasConsumer *)handle instanceForProbe];}return 0;}

extern "C" bool rastera_shared_pending_read_probe(void *handle){if(@available(macOS 14.0,*)){return [(__bridge SharedCanvasConsumer *)handle pendingReadForProbe];}return false;}

extern "C" void rastera_shared_sync_window_size(void *handle){if(@available(macOS 14.0,*)){[(__bridge SharedCanvasConsumer *)handle syncWindowSize];}}
