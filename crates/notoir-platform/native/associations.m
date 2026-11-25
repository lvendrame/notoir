// Runs before Rust main only in an explicitly requested helper child.
#import <Cocoa/Cocoa.h>
#import <CoreServices/CoreServices.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

__attribute__((constructor)) static void notoirAssociationsEntry(void) {
    @autoreleasepool {
        NSArray<NSString *> *args = NSProcessInfo.processInfo.arguments;
        if (args.count != 3 || ![args[1] isEqualToString:@"--notoir-internal-associations"]) return;
        @try {
            NSError *error = nil;
            NSData *data = [NSData dataWithContentsOfFile:[args[2] stringByAppendingPathComponent:@"request.json"]];
            NSDictionary *request = data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:&error] : nil;
            if (!request) @throw [NSException exceptionWithName:@"Request" reason:error.localizedDescription ?: @"Missing association request" userInfo:nil];
            NSString *action = request[@"action"];
            NSString *own = @"dev.notoir.Notoir";
            if ([action isEqualToString:@"inspect"]) {
                NSMutableArray *rows = [NSMutableArray array];
                for (NSString *extension in request[@"extensions"]) {
                    NSString *type = [UTType typeWithFilenameExtension:extension].identifier;
                    if (!type) type = @"public.plain-text";
                    NSString *handler = CFBridgingRelease(LSCopyDefaultRoleHandlerForContentType((__bridge CFStringRef)type, kLSRolesAll));
                    [rows addObject:@{@"extension":extension,@"content_type":type,@"handler":handler ?: NSNull.null,@"is_notoir":((handler && [handler caseInsensitiveCompare:own] == NSOrderedSame) ? @YES : @NO)}];
                }
                NSData *output = [NSJSONSerialization dataWithJSONObject:rows options:0 error:&error];
                if (!output) @throw [NSException exceptionWithName:@"Response" reason:error.localizedDescription userInfo:nil];
                fwrite(output.bytes, 1, output.length, stdout);
            } else if ([action isEqualToString:@"register"]) {
                NSString *bundle = request[@"executable"];
                while (bundle.length > 1 && ![bundle.pathExtension isEqualToString:@"app"]) bundle = bundle.stringByDeletingLastPathComponent;
                if (![bundle.pathExtension isEqualToString:@"app"] || ![[NSBundle bundleWithPath:bundle].bundleIdentifier isEqualToString:own])
                    @throw [NSException exceptionWithName:@"Bundle" reason:@"Install and run Notoir.app before managing file associations." userInfo:nil];
                OSStatus status = LSRegisterURL((__bridge CFURLRef)[NSURL fileURLWithPath:bundle], true);
                if (status != noErr) @throw [NSException exceptionWithName:@"Register" reason:[NSString stringWithFormat:@"Launch Services registration failed (%d)", (int)status] userInfo:nil];
            } else if ([action isEqualToString:@"change"]) {
                NSString *type = request[@"content_type"];
                NSString *handler = request[@"handler"] == NSNull.null ? nil : request[@"handler"];
                if (handler && ![NSWorkspace.sharedWorkspace URLForApplicationWithBundleIdentifier:handler]) handler = nil;
                if (!handler) {
                    NSArray *candidates = CFBridgingRelease(LSCopyAllRoleHandlersForContentType((__bridge CFStringRef)type, kLSRolesAll));
                    for (NSString *candidate in candidates) {
                        if ([candidate caseInsensitiveCompare:own] != NSOrderedSame && [NSWorkspace.sharedWorkspace URLForApplicationWithBundleIdentifier:candidate]) { handler = candidate; break; }
                    }
                }
                if (!handler) @throw [NSException exceptionWithName:@"Fallback" reason:@"No replacement application is available. Choose one in Finder using Get Info → Open with." userInfo:nil];
                OSStatus status = LSSetDefaultRoleHandlerForContentType((__bridge CFStringRef)type, kLSRolesAll, (__bridge CFStringRef)handler);
                if (status != noErr) @throw [NSException exceptionWithName:@"Change" reason:[NSString stringWithFormat:@"Launch Services could not set the default (%d)", (int)status] userInfo:nil];
            } else @throw [NSException exceptionWithName:@"Action" reason:@"Unknown association action" userInfo:nil];
            exit(0);
        } @catch (NSException *error) {
            fprintf(stderr, "%s\n", error.reason.UTF8String);
            exit(1);
        }
    }
}
