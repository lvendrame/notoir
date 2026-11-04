// Loaded only into the disposable debug app by live-native-menus.py.
// Exercise real AppKit menu tracking without requiring Accessibility access.
#import <Cocoa/Cocoa.h>

static NSMutableDictionary *results;
static NSString *output;
static BOOL tracking;
static BOOL finished;

static void saveResults(void) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:results options:NSJSONWritingPrettyPrinted error:nil];
    [data writeToFile:output atomically:YES];
}

static void fail(NSString *reason) {
    if (finished) return;
    finished = YES;
    results[@"error"] = reason;
    saveResults();
    [NSApp.mainMenu cancelTracking];
    for (NSMenuItem *item in NSApp.mainMenu.itemArray) [item.submenu cancelTracking];
}

static void later(NSTimeInterval delay, void (^action)(void)) {
    NSTimer *timer = [NSTimer timerWithTimeInterval:delay repeats:NO block:^(NSTimer *timer) { action(); }];
    [NSRunLoop.mainRunLoop addTimer:timer forMode:NSRunLoopCommonModes];
}

static NSArray<NSWindow *> *editors(void) {
    NSMutableArray *windows = [NSMutableArray array];
    for (NSWindow *window in NSApp.windows) {
        if (window.visible && window.frame.size.width >= 640 && ![window isKindOfClass:NSPanel.class]) {
            [windows addObject:window];
        }
    }
    return windows;
}

static NSInteger indexOf(NSMenu *menu, NSString *title) {
    for (NSInteger i = 0; i < menu.numberOfItems; ++i) {
        if ([[menu itemAtIndex:i].title isEqualToString:title]) return i;
    }
    return -1;
}

static BOOL identicalItems(NSMenu *menu, NSArray<NSMenuItem *> *items) {
    if (menu.numberOfItems != items.count) return NO;
    for (NSUInteger i = 0; i < items.count; ++i) {
        if ([menu itemAtIndex:i] != items[i]) return NO;
    }
    return YES;
}

static BOOL holdMenu(NSMenu *menu, NSView *view, NSString *key) {
    __block BOOL checked = NO;
    __block BOOL stable = NO;
    NSMenu *bar = NSApp.mainMenu;
    NSArray *topItems = [bar.itemArray copy];
    NSArray *menuItems = [menu.itemArray copy];
    tracking = NO;
    id begin = [NSNotificationCenter.defaultCenter addObserverForName:NSMenuDidBeginTrackingNotification
        object:menu queue:nil usingBlock:^(NSNotification *note) { tracking = YES; }];
    id end = [NSNotificationCenter.defaultCenter addObserverForName:NSMenuDidEndTrackingNotification
        object:menu queue:nil usingBlock:^(NSNotification *note) { tracking = NO; }];
    later(1.5, ^{
        checked = YES;
        stable = tracking && NSApp.mainMenu == bar && identicalItems(bar, topItems) && identicalItems(menu, menuItems);
        [menu cancelTracking];
    });
    [menu popUpMenuPositioningItem:nil atLocation:NSMakePoint(70, 70) inView:view];
    [NSNotificationCenter.defaultCenter removeObserver:begin];
    [NSNotificationCenter.defaultCenter removeObserver:end];
    if (!checked || !stable) {
        fail([NSString stringWithFormat:@"%@ closed early or its native items were replaced", key]);
        return NO;
    }
    results[key] = @YES;
    saveResults();
    return YES;
}

static void verifyZeroWindows(void) {
    if (finished) return;
    if (editors().count != 0) { fail(@"Editor windows did not close"); return; }
    results[@"zero_windows"] = @YES;
    NSMenu *appMenu = [NSApp.mainMenu itemAtIndex:0].submenu;
    NSInteger quit = indexOf(appMenu, @"Quit Notoir");
    if (quit < 0 || ![appMenu itemAtIndex:quit].enabled) { fail(@"Quit is unavailable without an editor"); return; }
    if (!holdMenu(appMenu, nil, @"app_menu_tracks_without_windows")) return;
    results[@"quit_dispatched"] = @YES;
    finished = YES;
    saveResults();
    [appMenu performActionForItemAtIndex:quit];
}

static void verifyNewWindow(NSUInteger before) {
    if (finished) return;
    if (editors().count != before + 1) { fail(@"Native New Window selection did not create an editor"); return; }
    results[@"native_new_window_action"] = @YES;
    saveResults();
    // Actual native close requests exercise the same zero-window state as the report.
    for (NSWindow *window in editors()) [window performClose:nil];
    later(2.0, ^{ verifyZeroWindows(); });
}

static void startProbe(void) {
    if (finished) return;
    NSArray<NSWindow *> *windows = editors();
    if (windows.count == 0) { fail(@"No native editor window"); return; }
    NSMenu *bar = NSApp.mainMenu;
    NSInteger fileIndex = indexOf(bar, @"File");
    if (fileIndex < 0) { fail(@"No native File menu"); return; }
    NSMenu *file = [bar itemAtIndex:fileIndex].submenu;
    NSInteger newWindow = indexOf(file, @"New Window");
    if (newWindow < 0 || ![file itemAtIndex:newWindow].enabled) { fail(@"New Window is unavailable"); return; }
    // Keep strong references so an allocator cannot conceal replacement by reusing addresses.
    NSArray *topItems = [bar.itemArray copy];
    NSArray *fileItems = [file.itemArray copy];
    later(2.0, ^{
        if (NSApp.mainMenu != bar || !identicalItems(bar, topItems) || !identicalItems(file, fileItems)) {
            fail(@"Editor frames replaced native menu objects"); return;
        }
        results[@"native_items_stable_across_frames"] = @YES;
        if (!holdMenu(file, windows.firstObject.contentView, @"file_menu_tracks_with_editor")) return;
        NSMenu *appMenu = [NSApp.mainMenu itemAtIndex:0].submenu;
        if (!holdMenu(appMenu, windows.firstObject.contentView, @"app_menu_tracks_with_editor")) return;
        [file performActionForItemAtIndex:newWindow];
        later(2.0, ^{ verifyNewWindow(windows.count); });
    });
}

__attribute__((constructor)) static void installProbe(void) {
    @autoreleasepool {
        NSDictionary *env = NSProcessInfo.processInfo.environment;
        output = env[@"NOTOIR_MENU_PROBE_OUTPUT"];
        if (!output || ![env[@"NOTOIR_TEST_COMMANDS"] isEqualToString:@"1"]) return;
        results = [NSMutableDictionary dictionary];
        [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationDidFinishLaunchingNotification
            object:nil queue:nil usingBlock:^(NSNotification *note) {
                later(3.0, ^{ startProbe(); });
            }];
    }
}
