#import <Cocoa/Cocoa.h>
#import <ApplicationServices/ApplicationServices.h>
// Normal NSWorkspace/NSRunningApplication lifecycle only. Never force-terminate.
int main(int argc,const char **argv){@autoreleasepool{
 NSString *command=argc>1?@(argv[1]):@"list";
 NSArray<NSRunningApplication*> *apps=[[NSWorkspace sharedWorkspace] runningApplications];NSMutableArray *rows=[NSMutableArray array];
 for(NSRunningApplication *app in apps){if(![app.bundleIdentifier isEqualToString:@"org.rastera.desktop.dev"])continue;
 [rows addObject:@{@"pid":@(app.processIdentifier),@"bundle":app.bundleURL.path?:@"",@"executable":app.executableURL.path?:@""}];
 if([command isEqualToString:@"quit"]||[command isEqualToString:@"cmdq"]){
 if([command isEqualToString:@"cmdq"]){[app activateWithOptions:0];[[NSRunLoop currentRunLoop]runUntilDate:[NSDate dateWithTimeIntervalSinceNow:.2]];CGEventRef down=CGEventCreateKeyboardEvent(NULL,12,true),up=CGEventCreateKeyboardEvent(NULL,12,false);CGEventSetFlags(down,kCGEventFlagMaskCommand);CGEventSetFlags(up,kCGEventFlagMaskCommand);CGEventPostToPid(app.processIdentifier,down);CGEventPostToPid(app.processIdentifier,up);CFRelease(down);CFRelease(up);}
 else if(![app terminate]){fprintf(stderr,"Rastera refused normal quit (PID %d)\n",app.processIdentifier);return 2;}NSDate *limit=[NSDate dateWithTimeIntervalSinceNow:20];while(!app.terminated&&limit.timeIntervalSinceNow>0)[[NSRunLoop currentRunLoop]runUntilDate:[NSDate dateWithTimeIntervalSinceNow:.05]];if(!app.terminated){fprintf(stderr,"Rastera did not finish normal shutdown; no files replaced\n");return 3;}}
 }
 NSData *data=[NSJSONSerialization dataWithJSONObject:rows options:NSJSONWritingPrettyPrinted error:nil];puts([[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding].UTF8String);return 0;
}}
