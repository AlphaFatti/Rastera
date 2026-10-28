#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
int main(int argc, const char **argv) { @autoreleasepool {
    int pid = argc>1 ? atoi(argv[1]) : 0;
    NSMutableArray *result=[NSMutableArray array];
    NSArray *windows=CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionAll,kCGNullWindowID));
    for (NSDictionary *w in windows) {
        if ([w[(__bridge NSString*)kCGWindowOwnerPID] intValue]!=pid) continue;
        [result addObject:@{@"id":w[(__bridge NSString*)kCGWindowNumber], @"title":w[(__bridge NSString*)kCGWindowName] ?: @"unavailable", @"bounds":w[(__bridge NSString*)kCGWindowBounds], @"onscreen":w[(__bridge NSString*)kCGWindowIsOnscreen] ?: @NO}];
    }
    NSDictionary *data=@{@"screen_capture_allowed":@(CGPreflightScreenCaptureAccess()),@"windows":result};
    NSData *json=[NSJSONSerialization dataWithJSONObject:data options:NSJSONWritingPrettyPrinted error:nil];
    fwrite(json.bytes,1,json.length,stdout);
} }
