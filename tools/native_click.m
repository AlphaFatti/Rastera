// Diagnostic driver: post one click only inside a named window owned by the given PID.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#include <unistd.h>
int main(int argc,const char **argv) { @autoreleasepool {
    if(argc!=5) return 2;
    if(!CGPreflightPostEventAccess()) { fprintf(stderr,"Accessibility event-posting access unavailable\n"); return 4; }
    int pid=atoi(argv[1]); NSString *title=@(argv[2]);
    CGPoint point=CGPointMake(strtod(argv[3],NULL),strtod(argv[4],NULL));
    BOOL allowed=NO;
    NSArray *windows=CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly,kCGNullWindowID));
    for(NSDictionary *w in windows) {
        if([w[(__bridge NSString *)kCGWindowOwnerPID] intValue]!=pid || ![w[(__bridge NSString *)kCGWindowName] isEqual:title]) continue;
        CGRect bounds; if(CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)w[(__bridge NSString *)kCGWindowBounds],&bounds) && CGRectContainsPoint(bounds,point)) allowed=YES;
    }
    if(!allowed) { fprintf(stderr,"Click rejected: point not in owned named window\n"); return 3; }
    CGEventRef down=CGEventCreateMouseEvent(NULL,kCGEventLeftMouseDown,point,kCGMouseButtonLeft);
    CGEventRef up=CGEventCreateMouseEvent(NULL,kCGEventLeftMouseUp,point,kCGMouseButtonLeft);
    CGEventPost(kCGHIDEventTap,down); usleep(50000); CGEventPost(kCGHIDEventTap,up);
    CFRelease(down); CFRelease(up);
} }
