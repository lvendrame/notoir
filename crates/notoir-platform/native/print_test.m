// Headless PDF regression for the same view and accessory used by the dialog.
#import "print.m"
#import <PDFKit/PDFKit.h>

static int savePDF(NotoirPrintView *view, NSString *path, BOOL rangeOnly) {
    NSPrintInfo *info = [NSPrintInfo.sharedPrintInfo copy];
    info.paperSize = NSMakeSize(320, 320);
    info.leftMargin = info.rightMargin = info.topMargin = info.bottomMargin = 24;
    info.verticallyCentered = NO;
    info.jobDisposition = NSPrintSaveJob;
    info.dictionary[NSPrintJobSavingURL] = [NSURL fileURLWithPath:path];
    if (rangeOnly) {
        info.dictionary[NSPrintAllPages] = @NO;
        info.dictionary[NSPrintFirstPage] = @2;
        info.dictionary[NSPrintLastPage] = @3;
    }
    NSPrintOperation *op = [NSPrintOperation printOperationWithView:view printInfo:info];
    op.showsPrintPanel = NO;
    op.showsProgressPanel = NO;
    if (![op runOperation]) return 1;
    PDFDocument *pdf = [[PDFDocument alloc] initWithURL:[NSURL fileURLWithPath:path]];
    if (!pdf || pdf.pageCount < (rangeOnly ? 2 : 5)) return 2;
    if (rangeOnly && pdf.pageCount != 2) return 3;
    if (!rangeOnly && ![pdf.string containsString:@"END-149"]) return 4;
    // Some system font PDF maps use compatibility ideographs.
    if (!rangeOnly && ![pdf.string.precomposedStringWithCompatibilityMapping containsString:@"日本語"]) return 5;
    if (!rangeOnly) {
        NSImage *image = [[pdf pageAtIndex:0] thumbnailOfSize:NSMakeSize(960, 960) forBox:kPDFDisplayBoxMediaBox];
        NSBitmapImageRep *bitmap = [NSBitmapImageRep imageRepWithData:image.TIFFRepresentation];
        NSData *png = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        [png writeToFile:[path stringByAppendingString:@".png"] atomically:YES];
    }
    return 0;
}
int main(int argc, char **argv) {
    @autoreleasepool {
        if (argc != 2) return 10;
        [NSApplication sharedApplication];
        NSMutableString *text = [NSMutableString string];
        for (int i = 0; i < 150; i++)
            [text appendFormat:@"    日本語 😀 line %d with a long line that must wrap within narrow paper margins. END-%d\n\n", i, i];
        NotoirPrintView *view = [[NotoirPrintView alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)];
        view.text = text;
        NSString *directory = [NSString stringWithUTF8String:argv[1]];
        int result = savePDF(view, [directory stringByAppendingPathComponent:@"all.pdf"], NO);
        if (result) return 20 + result;
        result = savePDF(view, [directory stringByAppendingPathComponent:@"range.pdf"], YES);
        if (result) return 30 + result;
        NotoirPrintSelection *accessory = [[NotoirPrintSelection alloc] init];
        accessory.textView = view;
        accessory.fullText = text;
        accessory.selectedText = @"selected 日本語";
        accessory.selectionOnly = YES;
        if (![view.text isEqualToString:accessory.selectedText]) return 40;
        accessory.selectionOnly = NO;
        if (![view.text isEqualToString:text]) return 41;
        return 0;
    }
}
