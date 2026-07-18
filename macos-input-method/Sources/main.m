#import <Cocoa/Cocoa.h>
#import <InputMethodKit/InputMethodKit.h>

#import "SpickInputController.h"
#import "SpickInputSourceInspection.h"
#import "SpickPeerIdentity.h"
#import "SpickWireProtocol.h"

#include <stdio.h>

// A deliberately read-only command used by compatibility preflight. It calls
// only TIS inspection APIs and returns before NSApplication or the insertion
// broker is created.
static int SpickPrintInputSourceState(void) {
    const SpickInputSourceState state = SpickInspectInputSourceState(
        "app.spick.desktop.input-method");
    switch (state) {
        case SpickInputSourceMissing:
            puts("missing");
            return EXIT_SUCCESS;
        case SpickInputSourceDisabled:
            puts("disabled");
            return EXIT_SUCCESS;
        case SpickInputSourceEnabled:
            puts("enabled");
            return EXIT_SUCCESS;
        case SpickInputSourceSelected:
            puts("selected");
            return EXIT_SUCCESS;
        case SpickInputSourceInvalid:
        default:
            fputs("input source state is invalid\n", stderr);
            return EXIT_FAILURE;
    }
}

@interface SpickApplicationDelegate : NSObject <NSApplicationDelegate>
@property(nonatomic, strong) IMKServer *inputMethodServer;
@end

@implementation SpickApplicationDelegate

- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender {
    (void)sender;
    @try {
        return [self.inputMethodServer paletteWillTerminate] ? NSTerminateNow
                                                             : NSTerminateCancel;
    } @catch (__unused NSException *exception) {
        return NSTerminateCancel;
    }
}

@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc == 2 && strcmp(argv[1], "--protocol-self-test") == 0) {
            return SpickRunWireProtocolSelfTests() && SpickRunPeerIdentitySelfTests()
                       ? EXIT_SUCCESS
                       : EXIT_FAILURE;
        }
        if (argc == 2 && strcmp(argv[1], "--peer-auth-runtime-self-test") == 0) {
            return SpickRunPeerIdentityRuntimeSelfTest(
                       "app.spick.desktop.input-method")
                       ? EXIT_SUCCESS
                       : EXIT_FAILURE;
        }
        if (argc == 2 && strcmp(argv[1], "--print-peer-auth-mode") == 0) {
            puts(SpickPeerAuthenticationAllowsUnsafeDevelopment()
                     ? "unsafe-adhoc"
                     : "secure");
            return EXIT_SUCCESS;
        }
        if (argc == 2 && strcmp(argv[1], "--print-input-source-state") == 0) {
            return SpickPrintInputSourceState();
        }
        if (argc != 1) {
            fputs("unknown Spick Input command\n", stderr);
            return 64;
        }

        NSApplication *application = NSApplication.sharedApplication;
        IMKServer *server = [[IMKServer alloc]
            initWithName:@"app.spick.input-method.connection"
        bundleIdentifier:@"app.spick.desktop.input-method"];
        if (server == nil) {
            return EXIT_FAILURE;
        }
        SpickApplicationDelegate *delegate = [[SpickApplicationDelegate alloc] init];
        delegate.inputMethodServer = server;
        application.delegate = delegate;
        SpickStartInsertionBroker();
        [application run];
    }
    return EXIT_SUCCESS;
}
