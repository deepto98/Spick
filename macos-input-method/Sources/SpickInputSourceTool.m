#import <Carbon/Carbon.h>
#import <Foundation/Foundation.h>

static NSString *const SpickInputSourceIdentifier = @"app.spick.desktop.input-method";

static NSArray *SpickMatchingInputSources(void) {
    NSDictionary *properties = @{
        (__bridge NSString *)kTISPropertyInputSourceID : SpickInputSourceIdentifier,
    };
    CFArrayRef sources = TISCreateInputSourceList((__bridge CFDictionaryRef)properties, true);
    return CFBridgingRelease(sources) ?: @[];
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 3 || strcmp(argv[1], "register-and-select") != 0) {
            fputs("usage: spick-input-source-tool register-and-select <bundle>\n", stderr);
            return EXIT_FAILURE;
        }

        NSString *path = [[NSFileManager defaultManager]
            stringWithFileSystemRepresentation:argv[2]
                                  length:strlen(argv[2])];
        NSURL *bundleURL = [NSURL fileURLWithPath:path isDirectory:YES];
        OSStatus status = TISRegisterInputSource((__bridge CFURLRef)bundleURL);
        if (status != noErr) {
            fprintf(stderr, "could not register Spick Input (%d)\n", (int)status);
            return EXIT_FAILURE;
        }

        NSArray *sources = SpickMatchingInputSources();
        if (sources.count != 1) {
            fputs("macOS did not return one Spick Input source after registration\n", stderr);
            return EXIT_FAILURE;
        }
        TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
        status = TISEnableInputSource(source);
        if (status != noErr) {
            fprintf(stderr, "could not enable Spick Input (%d)\n", (int)status);
            return EXIT_FAILURE;
        }
        status = TISSelectInputSource(source);
        if (status != noErr) {
            fprintf(stderr, "could not select Spick Input (%d)\n", (int)status);
            return EXIT_FAILURE;
        }
        puts("Spick Input is registered, enabled, and selected as a palette input source.");
    }
    return EXIT_SUCCESS;
}
