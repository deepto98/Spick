#import <Cocoa/Cocoa.h>
#import <InputMethodKit/InputMethodKit.h>

#import "SpickWireProtocol.h"

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc == 2 && strcmp(argv[1], "--protocol-self-test") == 0) {
            return SpickRunWireProtocolSelfTests() ? EXIT_SUCCESS : EXIT_FAILURE;
        }

        NSApplication *application = NSApplication.sharedApplication;
        IMKServer *server = [[IMKServer alloc]
            initWithName:@"app.spick.input-method.connection"
        bundleIdentifier:@"app.spick.desktop.input-method"];
        if (server == nil) {
            return EXIT_FAILURE;
        }
        (void)server;
        [application run];
    }
    return EXIT_SUCCESS;
}
