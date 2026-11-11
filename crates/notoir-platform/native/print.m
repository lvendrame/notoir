// Native helper entry point. Runs only in the explicitly requested child process.
#import <Cocoa/Cocoa.h>
#include <unistd.h>

// Explicit pagination keeps line fragments intact and recomputes wrapping when
// the user changes paper size, orientation or the selection in the print panel.
@interface NotoirPrintView : NSView
@property NSString *text;
@property NSLayoutManager *layout;
@property NSTextStorage *storage;
@property NSMutableArray<NSValue *> *pages;
@end
@implementation NotoirPrintView
- (BOOL)isFlipped { return YES; }
- (BOOL)knowsPageRange:(NSRangePointer)range {
    NSPrintInfo *info = NSPrintOperation.currentOperation.printInfo;
    CGFloat width = MAX(1, info.paperSize.width - info.leftMargin - info.rightMargin);
    CGFloat height = MAX(1, info.paperSize.height - info.topMargin - info.bottomMargin);
    self.storage = [[NSTextStorage alloc] initWithString:self.text attributes:@{
        NSFontAttributeName: [NSFont userFixedPitchFontOfSize:10],
        NSForegroundColorAttributeName: NSColor.blackColor
    }];
    self.layout = [[NSLayoutManager alloc] init];
    [self.storage addLayoutManager:self.layout];
    NSTextContainer *container = [[NSTextContainer alloc] initWithContainerSize:NSMakeSize(width, CGFLOAT_MAX)];
    container.lineFragmentPadding = 0;
    [self.layout addTextContainer:container];
    [self.layout ensureLayoutForTextContainer:container];
    self.pages = [NSMutableArray array];
    CGFloat start = 0;
    CGFloat end = 0;
    for (NSUInteger glyph = 0; glyph < self.layout.numberOfGlyphs;) {
        NSRange line;
        NSRect rect = [self.layout lineFragmentRectForGlyphAtIndex:glyph effectiveRange:&line];
        if (NSMaxY(rect) - start > height && rect.origin.y > start) {
            [self.pages addObject:[NSValue valueWithRect:NSMakeRect(0, start, width, rect.origin.y - start)]];
            start = rect.origin.y;
        }
        end = NSMaxY(rect);
        glyph = NSMaxRange(line);
    }
    // Empty documents still have one printable page.
    [self.pages addObject:[NSValue valueWithRect:NSMakeRect(0, start, width, MAX(1, end - start))]];
    self.frame = NSMakeRect(0, 0, width, MAX(height, end));
    *range = NSMakeRange(1, self.pages.count);
    return YES;
}
- (NSRect)rectForPage:(NSInteger)page { return self.pages[page - 1].rectValue; }
- (void)drawRect:(NSRect)dirty {
    NSRange glyphs = [self.layout glyphRangeForBoundingRect:dirty inTextContainer:self.layout.textContainers[0]];
    [self.layout drawGlyphsForGlyphRange:glyphs atPoint:NSZeroPoint];
}
@end

@interface NotoirPrintSelection : NSViewController <NSPrintPanelAccessorizing>
@property NotoirPrintView *textView;
@property NSString *fullText;
@property NSString *selectedText;
@property(nonatomic) BOOL selectionOnly;
@end
@implementation NotoirPrintSelection
- (void)setSelectionOnly:(BOOL)value {
    _selectionOnly = value;
    self.textView.text = value ? self.selectedText : self.fullText;
}
- (NSArray<NSDictionary<NSPrintPanelAccessorySummaryKey, NSString *> *> *)localizedSummaryItems {
    return @[];
}
- (NSSet<NSString *> *)keyPathsForValuesAffectingPreview {
    return [NSSet setWithObject:@"selectionOnly"];
}
@end

static NSString *readText(NSString *directory, NSString *name, BOOL required) {
    NSError *error = nil;
    NSString *text = [NSString stringWithContentsOfFile:[directory stringByAppendingPathComponent:name]
        encoding:NSUTF8StringEncoding error:&error];
    if (!text && required) {
        fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
        exit(1);
    }
    return text;
}

__attribute__((constructor)) static void notoirPrintEntry(void) {
    @autoreleasepool {
        NSArray<NSString *> *args = NSProcessInfo.processInfo.arguments;
        if (args.count != 3 || ![args[1] isEqualToString:@"--notoir-internal-print"]) return;
        @try {
            NSString *directory = args[2];
            NSString *text = readText(directory, @"text", YES);
            NSString *selection = readText(directory, @"selection", NO);
            NSString *title = readText(directory, @"title", YES);
            [NSApplication sharedApplication];
            [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
            [NSApp finishLaunching];
            [NSApp activateIgnoringOtherApps:YES];
            NSPrintInfo *info = [NSPrintInfo.sharedPrintInfo copy];
            info.horizontalPagination = NSPrintingPaginationModeAutomatic;
            info.verticalPagination = NSPrintingPaginationModeAutomatic;
            info.verticallyCentered = NO;
            info.dictionary[NSPrintAllPages] = @YES;
            NotoirPrintView *view = [[NotoirPrintView alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)];
            view.text = text;
            NSPrintOperation *operation = [NSPrintOperation printOperationWithView:view printInfo:info];
            operation.jobTitle = title;
            NSPrintPanel *panel = operation.printPanel;
            panel.options |= NSPrintPanelShowsPaperSize | NSPrintPanelShowsOrientation | NSPrintPanelShowsPageSetupAccessory;
            if (selection) {
                NotoirPrintSelection *accessory = [[NotoirPrintSelection alloc] init];
                accessory.textView = view;
                accessory.fullText = text;
                accessory.selectedText = selection;
                NSButton *checkbox = [NSButton checkboxWithTitle:readText(directory, @"selection-label", YES)
                    target:nil action:nil];
                checkbox.frame = NSMakeRect(0, 0, 300, 28);
                [checkbox bind:NSValueBinding toObject:accessory withKeyPath:@"selectionOnly" options:nil];
                accessory.view = checkbox;
                accessory.title = title;
                [panel addAccessoryController:accessory];
            }
            // Ask first so cancellation and a failed submission are distinct.
            if ([panel runModalWithPrintInfo:info] != NSModalResponseOK) exit(2);
            operation.showsPrintPanel = NO;
            if (![operation runOperation]) {
                fprintf(stderr, "The system print operation failed.\n");
                exit(1);
            }
            exit(0);
        } @catch (NSException *error) {
            fprintf(stderr, "%s\n", error.reason.UTF8String);
            exit(1);
        }
    }
}
