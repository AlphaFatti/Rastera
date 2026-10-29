// Process-scoped native window probe. No event posting, activation or cursor movement.
// Resolves AX objects directly by PID; same-named processes cannot alias by name.
#import <Foundation/Foundation.h>
#import <ApplicationServices/ApplicationServices.h>
#include <unistd.h>
static AXUIElementRef windowFor(pid_t pid,NSString *title) {
    AXUIElementRef app=AXUIElementCreateApplication(pid);CFArrayRef windows=NULL;
    AXUIElementCopyAttributeValue(app,kAXWindowsAttribute,(CFTypeRef *)&windows);CFRelease(app);
    AXUIElementRef result=NULL;
    for(id w in (__bridge NSArray *)windows) {
        CFTypeRef value=NULL;AXUIElementCopyAttributeValue((__bridge AXUIElementRef)w,kAXTitleAttribute,&value);
        pid_t owner=0;AXUIElementGetPid((__bridge AXUIElementRef)w,&owner);
        if(owner==pid && [(__bridge NSString *)value isEqual:title]) result=(AXUIElementRef)CFRetain((__bridge CFTypeRef)w);
        if(value)CFRelease(value);
        if(result)break;
    }
    if(windows)CFRelease(windows);return result;
}
int main(int argc,const char **argv) { @autoreleasepool {
    if(argc<4)return 2;
    pid_t pid=atoi(argv[1]);NSString *title=@(argv[2]),*mode=@(argv[3]);
    AXUIElementRef window=windowFor(pid,title);if(!window){fprintf(stderr,"Owned AX window unavailable\n");return 3;}
    AXError error=kAXErrorSuccess;
    if([mode isEqual:@"state"]) {
        CFTypeRef value=NULL;error=AXUIElementCopyAttributeValue(window,kAXMinimizedAttribute,&value);
        if(error==kAXErrorSuccess){printf("%d\n",CFBooleanGetValue((CFBooleanRef)value));CFRelease(value);}
    } else if([mode isEqual:@"minimize"]||[mode isEqual:@"close"]) {
        CFTypeRef button=NULL;error=AXUIElementCopyAttributeValue(window,[mode isEqual:@"close"]?kAXCloseButtonAttribute:kAXMinimizeButtonAttribute,&button);
        if(error==kAXErrorSuccess){error=AXUIElementPerformAction((AXUIElementRef)button,kAXPressAction);CFRelease(button);}
    } else if([mode isEqual:@"restore"]) {error=AXUIElementSetAttributeValue(window,kAXMinimizedAttribute,kCFBooleanFalse);
    } else if([mode isEqual:@"layout"]&&argc==8) {
        CGPoint p={atof(argv[4]),atof(argv[5])};CGSize s={atof(argv[6]),atof(argv[7])};
        AXValueRef position=AXValueCreate(kAXValueCGPointType,&p),size=AXValueCreate(kAXValueCGSizeType,&s);
        error=AXUIElementSetAttributeValue(window,kAXPositionAttribute,position);
        if(error==kAXErrorSuccess)error=AXUIElementSetAttributeValue(window,kAXSizeAttribute,size);
        CFRelease(position);CFRelease(size);
    } else {CFRelease(window);return 2;}
    if(window)CFRelease(window);
    if(error!=kAXErrorSuccess){fprintf(stderr,"AX operation failed: %d\n",error);return 5;}return 0;
} }
