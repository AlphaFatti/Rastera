// Included in shared_canvas.mm. All discovery, recovery and window operations are main-thread only.
#import <IOKit/IOKitLib.h>
static char outputDisplayKey;
static NSMutableArray<NSWindow *> *identificationWindows=nil;
static std::atomic<bool> outputsDisableRequested{false};
static NSHashTable<NSWindow *> *OutputWindows(){static NSHashTable<NSWindow *> *windows=[NSHashTable weakObjectsHashTable];return windows;}
static void HideOutputWindows(){for(NSWindow *window in OutputWindows()) [window orderOut:nil];}
static void DisableOutputWindows(){HideOutputWindows();outputsDisableRequested=true;}
static NSString *DisplayIdentity(NSScreen *screen){
    CGDirectDisplayID number=[screen.deviceDescription[@"NSScreenNumber"] unsignedIntValue];
    CFUUIDRef uuid=CGDisplayCreateUUIDFromDisplayID(number);if(!uuid)return @"";
    NSString *text=CFBridgingRelease(CFUUIDCreateString(kCFAllocatorDefault,uuid));CFRelease(uuid);return text;
}
static NSScreen *ResolveDisplay(NSString *identity){NSScreen *found=nil;for(NSScreen *screen in NSScreen.screens){if([DisplayIdentity(screen) isEqual:identity]){if(found)return nil;found=screen;}}return found;}
static void InstallOutputRecovery(){
    static id screenObserver=nil;
    if(!screenObserver)screenObserver=[NSNotificationCenter.defaultCenter addObserverForName:NSApplicationDidChangeScreenParametersNotification object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note){
        for(NSWindow *window in OutputWindows().allObjects){NSString *identity=objc_getAssociatedObject(window,&outputDisplayKey);if(!identity)continue;NSScreen *screen=ResolveDisplay(identity);if(!screen){[window close];}else{[window setFrame:screen.frame display:YES];[(SharedCanvasView *)window.contentView resizeDrawable];}}
    }];
    static id monitor=nil;
    if(!monitor)monitor=[NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown handler:^NSEvent *(NSEvent *event){
        if(event.keyCode==53&&(event.modifierFlags&(NSEventModifierFlagCommand|NSEventModifierFlagShift))==(NSEventModifierFlagCommand|NSEventModifierFlagShift)){DisableOutputWindows();return nil;}return event;
    }];
}
extern "C" void rastera_outputs_hide_all(){NSCAssert(NSThread.isMainThread,@"Hide on main");HideOutputWindows();}
extern "C" void rastera_outputs_disable_all(){NSCAssert(NSThread.isMainThread,@"Disable on main");DisableOutputWindows();}
extern "C" bool rastera_outputs_take_disable(){NSCAssert(NSThread.isMainThread,@"Recovery on main");InstallOutputRecovery();return outputsDisableRequested.exchange(false);}
extern "C" char *rastera_displays_json(){
    NSCAssert(NSThread.isMainThread,@"Display discovery on main");
    NSMutableArray *displays=[NSMutableArray array];
    for(NSScreen *screen in NSScreen.screens){
        NSRect frame=screen.frame;NSSize pixels=[screen convertRectToBacking:frame].size;
        [displays addObject:@{@"id":DisplayIdentity(screen),@"name":screen.localizedName?:@"Display",@"pixels":@[@(llround(pixels.width)),@(llround(pixels.height))],@"frame":@[@(frame.origin.x),@(frame.origin.y),@(frame.size.width),@(frame.size.height)],@"builtin":@(bool(CGDisplayIsBuiltin([screen.deviceDescription[@"NSScreenNumber"] unsignedIntValue]))),@"number":screen.deviceDescription[@"NSScreenNumber"]}];
    }
    NSString *machine=@"";io_service_t service=IOServiceGetMatchingService(kIOMainPortDefault,IOServiceMatching("IOPlatformExpertDevice"));
    if(service){CFTypeRef value=IORegistryEntryCreateCFProperty(service,CFSTR("IOPlatformUUID"),kCFAllocatorDefault,0);if(value){id object=CFBridgingRelease(value);if([object isKindOfClass:NSString.class])machine=object;}IOObjectRelease(service);}
    NSData *data=[NSJSONSerialization dataWithJSONObject:@{@"machine":machine,@"displays":displays} options:0 error:nil];
    return strdup([[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}
extern "C" void rastera_identify_displays(){
    NSCAssert(NSThread.isMainThread,@"Identify on main");
    // Small click-through labels, owned separately from canvas/native rendering resources.
    for(NSWindow *window in identificationWindows)[window orderOut:nil];identificationWindows=[NSMutableArray array];
    for(NSScreen *screen in NSScreen.screens){
        NSRect frame=screen.frame;NSSize pixels=[screen convertRectToBacking:frame].size;
        NSWindow *window=[[NSWindow alloc]initWithContentRect:NSMakeRect(NSMidX(frame)-260,NSMidY(frame)-85,520,170) styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
        window.releasedWhenClosed=NO;window.level=NSStatusWindowLevel+2;window.ignoresMouseEvents=YES;window.hidesOnDeactivate=NO;
        window.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorCanJoinAllApplications;
        float bg[4];rastera_theme_workspace(bg);window.backgroundColor=[NSColor colorWithSRGBRed:bg[0] green:bg[1] blue:bg[2] alpha:1];
        NSTextField *label=[NSTextField labelWithString:[NSString stringWithFormat:@"%@\n%.0f × %.0f px\nCG%@ · %@",screen.localizedName,pixels.width,pixels.height,screen.deviceDescription[@"NSScreenNumber"],[DisplayIdentity(screen) substringToIndex:std::min<NSUInteger>(8,DisplayIdentity(screen).length)]]];
        label.frame=NSMakeRect(10,10,500,150);label.alignment=NSTextAlignmentCenter;label.font=[NSFont systemFontOfSize:28 weight:NSFontWeightBold];label.textColor=NSColor.whiteColor;[window.contentView addSubview:label];[window orderFrontRegardless];[identificationWindows addObject:window];
    }
    NSArray *owned=[identificationWindows copy];dispatch_after(dispatch_time(DISPATCH_TIME_NOW,3*NSEC_PER_SEC),dispatch_get_main_queue(),^{for(NSWindow *window in owned)[window orderOut:nil];});
}
@interface RasteraCleanWindow : NSWindow
@end
@implementation RasteraCleanWindow
- (BOOL)canBecomeKeyWindow{return NO;}
- (BOOL)canBecomeMainWindow{return NO;}
@end

extern "C" uint32_t rastera_identify_visible_probe(){uint32_t count=0;for(NSWindow *window in identificationWindows)if(window.isVisible)++count;return count;}
