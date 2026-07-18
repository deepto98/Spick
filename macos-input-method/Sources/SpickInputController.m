#import "SpickInputController.h"

#import "SpickWireProtocol.h"

#include <Carbon/Carbon.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

static __weak SpickInputController *SpickActiveController;

@interface SpickInputController (Insertion)
- (SpickInsertStatus)handleInsertRequest:(SpickInsertRequest *)request;
@end

static BOOL SpickReadExactly(int descriptor, void *buffer, size_t length) {
    uint8_t *cursor = buffer;
    size_t remaining = length;
    while (remaining > 0) {
        const ssize_t count = read(descriptor, cursor, remaining);
        if (count == 0) {
            return NO;
        }
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            return NO;
        }
        cursor += count;
        remaining -= (size_t)count;
    }
    return YES;
}

static BOOL SpickWriteExactly(int descriptor, const void *buffer, size_t length) {
    const uint8_t *cursor = buffer;
    size_t remaining = length;
    while (remaining > 0) {
        const ssize_t count = write(descriptor, cursor, remaining);
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            return NO;
        }
        cursor += count;
        remaining -= (size_t)count;
    }
    return YES;
}

@interface SpickInsertionBroker : NSObject
@property(nonatomic) int listenerDescriptor;
@property(nonatomic) int lockDescriptor;
@property(nonatomic, copy) NSString *socketPath;
@end

@implementation SpickInsertionBroker

+ (instancetype)sharedBroker {
    static SpickInsertionBroker *broker;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
      broker = [[self alloc] init];
      broker.listenerDescriptor = -1;
      broker.lockDescriptor = -1;
    });
    return broker;
}

- (void)start {
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
      [self runListener];
    });
}

- (void)runListener {
    @autoreleasepool {
        NSString *temporaryDirectory = NSTemporaryDirectory();
        if (temporaryDirectory.length == 0) {
            return;
        }
        self.socketPath =
            [temporaryDirectory stringByAppendingPathComponent:@"app.spick.input-method.sock"];
        NSString *lockPath = [self.socketPath stringByAppendingString:@".lock"];
        const char *lockFile = lockPath.fileSystemRepresentation;
        self.lockDescriptor = open(lockFile, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
        if (self.lockDescriptor < 0 || flock(self.lockDescriptor, LOCK_EX | LOCK_NB) != 0) {
            return;
        }

        const char *socketFile = self.socketPath.fileSystemRepresentation;
        struct stat existing;
        if (lstat(socketFile, &existing) == 0) {
            if (!S_ISSOCK(existing.st_mode) || existing.st_uid != geteuid() ||
                unlink(socketFile) != 0) {
                return;
            }
        } else if (errno != ENOENT) {
            return;
        }

        struct sockaddr_un address = {0};
        address.sun_family = AF_UNIX;
        const size_t pathLength = strlen(socketFile);
        if (pathLength == 0 || pathLength >= sizeof(address.sun_path)) {
            return;
        }
        memcpy(address.sun_path, socketFile, pathLength + 1);

        self.listenerDescriptor = socket(AF_UNIX, SOCK_STREAM, 0);
        if (self.listenerDescriptor < 0) {
            return;
        }
        (void)fcntl(self.listenerDescriptor, F_SETFD, FD_CLOEXEC);
        if (bind(self.listenerDescriptor, (const struct sockaddr *)&address, sizeof(address)) != 0 ||
            chmod(socketFile, 0600) != 0 || listen(self.listenerDescriptor, 8) != 0) {
            close(self.listenerDescriptor);
            self.listenerDescriptor = -1;
            (void)unlink(socketFile);
            return;
        }

        for (;;) {
            const int client = accept(self.listenerDescriptor, NULL, NULL);
            if (client < 0) {
                if (errno == EINTR) {
                    continue;
                }
                break;
            }
            @autoreleasepool {
                [self handleClient:client];
            }
            close(client);
        }
        (void)unlink(socketFile);
    }
}

- (void)handleClient:(int)client {
    uid_t peerUser = 0;
    gid_t peerGroup = 0;
    if (getpeereid(client, &peerUser, &peerGroup) != 0 || peerUser != geteuid()) {
        return;
    }
    (void)peerGroup;
    const int enabled = 1;
    (void)setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof(enabled));
    const struct timeval timeout = {.tv_sec = 1, .tv_usec = 0};
    (void)setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    (void)setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));

    NSMutableData *frame = [NSMutableData dataWithLength:SpickRequestHeaderLength];
    if (!SpickReadExactly(client, frame.mutableBytes, SpickRequestHeaderLength)) {
        return;
    }
    NSUInteger frameLength = 0;
    uint64_t requestID = 0;
    if (!SpickRequestFrameLengthFromHeader(frame, &frameLength, &requestID)) {
        return;
    }
    [frame setLength:frameLength];
    const NSUInteger payloadLength = frameLength - SpickRequestHeaderLength;
    if (!SpickReadExactly(client,
                          (uint8_t *)frame.mutableBytes + SpickRequestHeaderLength,
                          payloadLength)) {
        return;
    }

    SpickInsertRequest *request = SpickDecodeInsertRequest(frame);
    __block SpickInsertStatus status = SpickInsertStatusInvalidRequest;
    if (request != nil) {
        dispatch_sync(dispatch_get_main_queue(), ^{
          SpickInputController *controller = SpickActiveController;
          status = controller == nil ? SpickInsertStatusNoActiveClient
                                     : [controller handleInsertRequest:request];
        });
    }

    NSData *response = SpickEncodeResponse(status, requestID);
    (void)SpickWriteExactly(client, response.bytes, response.length);
}

@end

@implementation SpickInputController

+ (void)load {
    [[SpickInsertionBroker sharedBroker] start];
}

- (void)activateServer:(id)sender {
    [super activateServer:sender];
    SpickActiveController = self;
}

- (void)deactivateServer:(id)sender {
    if (SpickActiveController == self) {
        SpickActiveController = nil;
    }
    [super deactivateServer:sender];
}

- (void)inputControllerWillClose {
    if (SpickActiveController == self) {
        SpickActiveController = nil;
    }
    [super inputControllerWillClose];
}

- (SpickInsertStatus)handleInsertRequest:(SpickInsertRequest *)request {
    NSAssert(NSThread.isMainThread, @"InputMethodKit calls must stay on the main thread");
    if (IsSecureEventInputEnabled()) {
        return SpickInsertStatusSecureInput;
    }

    id<IMKTextInput, NSObject> client = self.client;
    if (client == nil) {
        return SpickInsertStatusNoActiveClient;
    }
    if (![client supportsUnicode]) {
        return SpickInsertStatusUnsupported;
    }
    NSString *bundleIdentifier = [client bundleIdentifier];
    if (bundleIdentifier == nil ||
        ![bundleIdentifier isEqualToString:request.bundleIdentifier]) {
        return SpickInsertStatusTargetMismatch;
    }

    NSRange selection = [client selectedRange];
    if (selection.location == NSNotFound || selection.length == NSNotFound) {
        return SpickInsertStatusUnsupported;
    }
    if (!NSEqualRanges(selection, request.selection)) {
        return SpickInsertStatusSelectionChanged;
    }
    NSRange markedRange = [client markedRange];
    if (markedRange.location != NSNotFound) {
        return SpickInsertStatusUnsupported;
    }
    if (request.text.length > NSUIntegerMax - selection.location) {
        return SpickInsertStatusInvalidRequest;
    }

    BOOL attempted = NO;
    @try {
        attempted = YES;
        [client insertText:request.text
            replacementRange:NSMakeRange(NSNotFound, NSNotFound)];
        const NSRange resultingSelection = [client selectedRange];
        const NSRange expectedSelection =
            NSMakeRange(selection.location + request.text.length, 0);
        return NSEqualRanges(resultingSelection, expectedSelection)
                   ? SpickInsertStatusConfirmed
                   : SpickInsertStatusDispatched;
    } @catch (__unused NSException *exception) {
        return attempted ? SpickInsertStatusDispatched : SpickInsertStatusInternalError;
    }
}

@end
