#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>
#import <Foundation/Foundation.h>

#include <fcntl.h>
#include <errno.h>
#include <limits.h>
#include <poll.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static NSString *const SpickInputSourceIdentifier = @"app.spick.desktop.input-method";

static NSArray *SpickMatchingInputSources(void) {
    NSDictionary *properties = @{
        (__bridge NSString *)kTISPropertyInputSourceID : SpickInputSourceIdentifier,
    };
    CFArrayRef sources = TISCreateInputSourceList((__bridge CFDictionaryRef)properties, true);
    return CFBridgingRelease(sources) ?: @[];
}

static BOOL SpickBooleanProperty(TISInputSourceRef source, CFStringRef key) {
    CFTypeRef value = TISGetInputSourceProperty(source, key);
    return value != NULL && CFGetTypeID(value) == CFBooleanGetTypeID() &&
           CFBooleanGetValue((CFBooleanRef)value);
}

static BOOL SpickSourceShapeIsValid(TISInputSourceRef source) {
    CFTypeRef category = TISGetInputSourceProperty(source, kTISPropertyInputSourceCategory);
    CFTypeRef bundleID = TISGetInputSourceProperty(source, kTISPropertyBundleID);
    return category != NULL && CFEqual(category, kTISCategoryPaletteInputSource) &&
           bundleID != NULL &&
           CFEqual(bundleID, (__bridge CFStringRef)SpickInputSourceIdentifier) &&
           SpickBooleanProperty(source, kTISPropertyInputSourceIsEnableCapable) &&
           SpickBooleanProperty(source, kTISPropertyInputSourceIsSelectCapable);
}

static BOOL SpickWaitForSourceState(BOOL expectedEnabled, BOOL expectedSelected) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:3.0];
    do {
        NSArray *sources = SpickMatchingInputSources();
        if (sources.count == 1) {
            TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
            if (SpickSourceShapeIsValid(source) &&
                SpickBooleanProperty(source, kTISPropertyInputSourceIsEnabled) ==
                    expectedEnabled &&
                SpickBooleanProperty(source, kTISPropertyInputSourceIsSelected) ==
                    expectedSelected) {
                return YES;
            }
        }
        [NSThread sleepForTimeInterval:0.05];
    } while ([deadline timeIntervalSinceNow] > 0);
    return NO;
}

static BOOL SpickBrokerAcceptsConnection(NSString *socketPath, NSDate *deadline) {
    const char *socketFile = socketPath.fileSystemRepresentation;
    const size_t pathLength = strlen(socketFile);
    struct sockaddr_un address = {0};
    if (pathLength == 0 || pathLength >= sizeof(address.sun_path)) {
        return NO;
    }
    address.sun_family = AF_UNIX;
    memcpy(address.sun_path, socketFile, pathLength + 1);

    const int descriptor = socket(AF_UNIX, SOCK_STREAM, 0);
    if (descriptor < 0) {
        return NO;
    }
    const int descriptorFlags = fcntl(descriptor, F_GETFD);
    const int statusFlags = fcntl(descriptor, F_GETFL);
    if (descriptorFlags < 0 || statusFlags < 0 ||
        fcntl(descriptor, F_SETFD, descriptorFlags | FD_CLOEXEC) != 0 ||
        fcntl(descriptor, F_SETFL, statusFlags | O_NONBLOCK) != 0) {
        close(descriptor);
        return NO;
    }

    BOOL connected =
        connect(descriptor, (const struct sockaddr *)&address, sizeof(address)) == 0;
    if (!connected && errno == EINPROGRESS) {
        for (;;) {
            const NSTimeInterval remaining = deadline.timeIntervalSinceNow;
            if (remaining <= 0) {
                break;
            }
            const double milliseconds = remaining * 1000.0;
            const int timeout =
                (int)MIN((uint64_t)milliseconds + 1, (uint64_t)INT_MAX);
            struct pollfd pollDescriptor = {
                .fd = descriptor,
                .events = POLLOUT,
                .revents = 0,
            };
            const int result = poll(&pollDescriptor, 1, timeout);
            if (result > 0) {
                int socketError = 0;
                socklen_t errorLength = sizeof(socketError);
                connected = (pollDescriptor.revents & POLLOUT) != 0 &&
                            getsockopt(descriptor, SOL_SOCKET, SO_ERROR, &socketError,
                                       &errorLength) == 0 &&
                            socketError == 0;
                break;
            }
            if (result == 0 || errno != EINTR) {
                break;
            }
        }
    }
    close(descriptor);
    return connected;
}

static BOOL SpickCanonicalBundleIsValid(NSString *path) {
    NSBundle *bundle = [NSBundle bundleWithPath:path];
    return bundle != nil &&
           [bundle.bundleIdentifier isEqualToString:SpickInputSourceIdentifier];
}

static BOOL SpickRunningApplicationsMatchPath(NSString *path) {
    NSString *expected = path.stringByStandardizingPath;
    NSArray<NSRunningApplication *> *applications =
        [NSRunningApplication runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier];
    for (NSRunningApplication *application in applications) {
        if (![application.bundleURL.path.stringByStandardizingPath isEqualToString:expected]) {
            return NO;
        }
    }
    return YES;
}

static BOOL SpickWaitForNoRunningApplication(void) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:3.0];
    do {
        if ([NSRunningApplication
                runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier].count == 0) {
            return YES;
        }
        [NSThread sleepForTimeInterval:0.05];
    } while ([deadline timeIntervalSinceNow] > 0);
    return NO;
}

static int SpickInspectInstall(NSString *path) {
    NSArray *sources = SpickMatchingInputSources();
    if (sources.count > 1) {
        fputs("multiple Spick Input sources are registered\n", stderr);
        return EXIT_FAILURE;
    }
    if (sources.count == 0) {
        if ([NSRunningApplication
                runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier].count != 0) {
            fputs("Spick Input is running without one registered source\n", stderr);
            return EXIT_FAILURE;
        }
        puts("missing");
        return EXIT_SUCCESS;
    }

    TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
    if (!SpickSourceShapeIsValid(source) || !SpickCanonicalBundleIsValid(path) ||
        !SpickRunningApplicationsMatchPath(path)) {
        fputs("the registered Spick Input source does not match the canonical installed bundle\n",
              stderr);
        return EXIT_FAILURE;
    }
    if (SpickBooleanProperty(source, kTISPropertyInputSourceIsSelected)) {
        puts("selected");
    } else if (SpickBooleanProperty(source, kTISPropertyInputSourceIsEnabled)) {
        puts("enabled");
    } else {
        puts("disabled");
    }
    return EXIT_SUCCESS;
}

static int SpickAssertSafeToReplace(NSString *path) {
    NSArray *sources = SpickMatchingInputSources();
    if (sources.count > 1) {
        fputs("multiple Spick Input sources appeared during installation\n", stderr);
        return EXIT_FAILURE;
    }
    if (sources.count == 1) {
        TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
        if (!SpickSourceShapeIsValid(source) || !SpickCanonicalBundleIsValid(path) ||
            SpickBooleanProperty(source, kTISPropertyInputSourceIsSelected) ||
            SpickBooleanProperty(source, kTISPropertyInputSourceIsEnabled)) {
            fputs("Spick Input is no longer disabled and safe to replace\n", stderr);
            return EXIT_FAILURE;
        }
    }
    if ([NSRunningApplication
            runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier].count != 0) {
        fputs("Spick Input started again before its bundle could be replaced\n", stderr);
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}

static BOOL SpickWaitForExpectedApplication(NSString *expectedPath) {
    NSString *socketPath =
        [NSTemporaryDirectory() stringByAppendingPathComponent:@"app.spick.input-method.sock"];
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:3.0];
    do {
        BOOL expectedProcessIsRunning = NO;
        NSArray<NSRunningApplication *> *applications =
            [NSRunningApplication runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier];
        for (NSRunningApplication *application in applications) {
            NSString *path = application.bundleURL.path.stringByStandardizingPath;
            if ([path isEqualToString:expectedPath.stringByStandardizingPath]) {
                expectedProcessIsRunning = YES;
                break;
            }
        }

        struct stat socketStatus = {0};
        const BOOL privateSocketIsReady =
            lstat(socketPath.fileSystemRepresentation, &socketStatus) == 0 &&
            S_ISSOCK(socketStatus.st_mode) && socketStatus.st_uid == geteuid() &&
            (socketStatus.st_mode & 0077) == 0;
        if (expectedProcessIsRunning && privateSocketIsReady &&
            SpickBrokerAcceptsConnection(socketPath, deadline)) {
            return YES;
        }
        [NSThread sleepForTimeInterval:0.05];
    } while ([deadline timeIntervalSinceNow] > 0);
    return NO;
}

static int SpickPrepareForInstall(NSString *path) {
    NSArray *sources = SpickMatchingInputSources();
    if (sources.count > 1) {
        fputs("multiple Spick Input sources are registered; remove legacy backup .app bundles first\n",
              stderr);
        return EXIT_FAILURE;
    }
    BOOL wasEnabled = NO;
    if (sources.count == 1) {
        TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
        if (!SpickSourceShapeIsValid(source) || !SpickCanonicalBundleIsValid(path) ||
            !SpickRunningApplicationsMatchPath(path)) {
            fputs("the existing Spick Input source does not match the canonical bundle\n",
                  stderr);
            return EXIT_FAILURE;
        }
        if (SpickBooleanProperty(source, kTISPropertyInputSourceIsSelected)) {
            OSStatus status = TISDeselectInputSource(source);
            if (status != noErr || !SpickWaitForSourceState(YES, NO)) {
                fprintf(stderr, "could not deselect Spick Input (%d)\n", (int)status);
                return EXIT_FAILURE;
            }
            sources = SpickMatchingInputSources();
            if (sources.count != 1 ||
                !SpickSourceShapeIsValid(
                    (__bridge TISInputSourceRef)sources.firstObject)) {
                fputs("Spick Input changed while it was being deselected\n", stderr);
                return EXIT_FAILURE;
            }
            source = (__bridge TISInputSourceRef)sources.firstObject;
        }
        wasEnabled = SpickBooleanProperty(source, kTISPropertyInputSourceIsEnabled);
    }

    if (sources.count == 1) {
        TISInputSourceRef source = (__bridge TISInputSourceRef)sources.firstObject;
        if (wasEnabled) {
            OSStatus status = TISDisableInputSource(source);
            if (status != noErr || !SpickWaitForSourceState(NO, NO)) {
                if (status == noErr) {
                    (void)TISEnableInputSource(source);
                }
                fprintf(stderr, "could not disable the existing Spick Input source (%d)\n",
                        (int)status);
                return EXIT_FAILURE;
            }
        }
    }

    NSArray<NSRunningApplication *> *applications =
        [NSRunningApplication runningApplicationsWithBundleIdentifier:SpickInputSourceIdentifier];
    for (NSRunningApplication *application in applications) {
        if (![application terminate]) {
            fputs("could not ask the running Spick Input process to stop\n", stderr);
            return EXIT_FAILURE;
        }
    }
    if (!SpickWaitForNoRunningApplication()) {
        fputs("Spick Input did not stop; no bundle files were changed\n", stderr);
        return EXIT_FAILURE;
    }
    return SpickAssertSafeToReplace(path);
}

static int SpickRegisterAndSetState(NSString *path, BOOL shouldSelect) {
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
    if (!SpickSourceShapeIsValid(source)) {
        fputs("the registered Spick Input source has unexpected capabilities\n", stderr);
        return EXIT_FAILURE;
    }
    status = TISEnableInputSource(source);
    if (status != noErr) {
        fprintf(stderr, "could not enable Spick Input (%d)\n", (int)status);
        return EXIT_FAILURE;
    }
    if (!SpickWaitForSourceState(YES, NO)) {
        fputs("macOS did not finish enabling Spick Input\n", stderr);
        return EXIT_FAILURE;
    }

    if (!shouldSelect) {
        puts("Spick Input is registered and enabled.");
        return EXIT_SUCCESS;
    }

    sources = SpickMatchingInputSources();
    if (sources.count != 1) {
        fputs("Spick Input changed while it was being enabled\n", stderr);
        return EXIT_FAILURE;
    }
    source = (__bridge TISInputSourceRef)sources.firstObject;
    if (!SpickSourceShapeIsValid(source)) {
        fputs("Spick Input changed capabilities while it was being enabled\n", stderr);
        return EXIT_FAILURE;
    }
    status = TISSelectInputSource(source);
    if (status != noErr) {
        fprintf(stderr, "could not select Spick Input (%d)\n", (int)status);
        return EXIT_FAILURE;
    }
    if (!SpickWaitForSourceState(YES, YES)) {
        fputs("macOS did not leave Spick Input enabled and selected\n", stderr);
        return EXIT_FAILURE;
    }
    if (!SpickWaitForExpectedApplication(path)) {
        fputs("the newly installed Spick Input broker did not become ready\n", stderr);
        return EXIT_FAILURE;
    }
    puts("Spick Input is registered, enabled, selected, and ready.");
    return EXIT_SUCCESS;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc == 3 && strcmp(argv[1], "inspect-install") == 0) {
            NSString *path = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:argv[2]
                                      length:strlen(argv[2])];
            return SpickInspectInstall(path);
        }
        if (argc == 3 && strcmp(argv[1], "prepare-install") == 0) {
            NSString *path = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:argv[2]
                                      length:strlen(argv[2])];
            return SpickPrepareForInstall(path);
        }
        if (argc == 3 && strcmp(argv[1], "assert-safe-to-replace") == 0) {
            NSString *path = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:argv[2]
                                      length:strlen(argv[2])];
            return SpickAssertSafeToReplace(path);
        }
        if (argc == 3 && strcmp(argv[1], "register-and-enable") == 0) {
            NSString *path = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:argv[2]
                                      length:strlen(argv[2])];
            return SpickRegisterAndSetState(path, NO);
        }
        if (argc != 3 || strcmp(argv[1], "register-and-select") != 0) {
            fputs("usage: spick-input-source-tool inspect-install <bundle> | "
                  "prepare-install <bundle> | assert-safe-to-replace <bundle> | "
                  "register-and-enable <bundle> | register-and-select <bundle>\n",
                  stderr);
            return EXIT_FAILURE;
        }

        NSString *path = [[NSFileManager defaultManager]
            stringWithFileSystemRepresentation:argv[2]
                                  length:strlen(argv[2])];
        return SpickRegisterAndSetState(path, YES);
    }
}
