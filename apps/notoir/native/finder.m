// Receive Finder's Open Documents event without replacing winit's delegate.
// Forward paths through Notoir's existing single-instance protocol.
#import <Cocoa/Cocoa.h>
@interface NotoirFinderEvents : NSObject
- (void)openDocuments:(NSAppleEventDescriptor *)event reply:(NSAppleEventDescriptor *)reply;
@end
@implementation NotoirFinderEvents
- (void)openDocuments:(NSAppleEventDescriptor *)event reply:(NSAppleEventDescriptor *)reply {
    NSAppleEventDescriptor *items = [event paramDescriptorForKeyword:keyDirectObject];
    NSMutableArray<NSString *> *arguments = [NSMutableArray arrayWithObject:@"--"];
    for (NSInteger index = 1; index <= items.numberOfItems; index++) {
        NSURL *url = [items descriptorAtIndex:index].fileURLValue;
        if (url.isFileURL && url.path) [arguments addObject:url.path];
    }
    if (arguments.count == 1) return;
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:NSProcessInfo.processInfo.arguments[0]];
    task.arguments = arguments;
    NSError *error = nil;
    if (![task launchAndReturnError:&error]) NSLog(@"Notoir: Finder open failed: %@", error);
}
@end
__attribute__((constructor)) static void registerFinderEvents(void) {
    @autoreleasepool {
        // Do not create NSApplication before winit initializes it. AppKit's
        // handlers are installed by the time willFinishLaunching is delivered.
        [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationWillFinishLaunchingNotification
            object:nil queue:nil usingBlock:^(NSNotification *notification) {
                static NotoirFinderEvents *handler;
                handler = [[NotoirFinderEvents alloc] init];
                [NSAppleEventManager.sharedAppleEventManager setEventHandler:handler
                    andSelector:@selector(openDocuments:reply:) forEventClass:kCoreEventClass
                    andEventID:kAEOpenDocuments];
            }];
    }
}
