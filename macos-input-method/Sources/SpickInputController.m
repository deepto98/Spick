#import "SpickInputController.h"

#import "SpickWireProtocol.h"

#include <Carbon/Carbon.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <stdatomic.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

static NSString *const SpickDesktopBundleIdentifier = @"app.spick.desktop";
static const uint64_t SpickLeaseLifetimeNanoseconds = 30ULL * 60 * NSEC_PER_SEC;
static const uint64_t SpickConnectionLifetimeNanoseconds = NSEC_PER_SEC;
static const NSUInteger SpickMaximumLiveLeases = 64;
static __weak SpickInputController *SpickActiveController;

@interface SpickInputController ()
@property(nonatomic) uint64_t activationGeneration;
@end

@interface SpickInputLease : NSObject
@property(nonatomic) uint64_t leaseID;
@property(nonatomic) uint64_t requestID;
@property(nonatomic) uint64_t expiresAtMonotonicNanoseconds;
@property(nonatomic) uint64_t activationGeneration;
@property(nonatomic, weak) SpickInputController *controller;
@property(nonatomic, copy) NSString *clientIdentifier;
@property(nonatomic, copy) NSString *bundleIdentifier;
@property(nonatomic) NSRange selection;
@end

@implementation SpickInputLease
@end

static NSMutableDictionary<NSNumber *, SpickInputLease *> *SpickLeaseStore(void) {
    static NSMutableDictionary<NSNumber *, SpickInputLease *> *leases;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
      leases = [NSMutableDictionary dictionary];
    });
    return leases;
}

static uint64_t SpickMonotonicNanoseconds(void) {
    struct timespec time = {0};
    if (clock_gettime(CLOCK_MONOTONIC, &time) != 0) {
        return 0;
    }
    return (uint64_t)time.tv_sec * NSEC_PER_SEC + (uint64_t)time.tv_nsec;
}

static BOOL SpickDeadlinePassed(uint64_t deadline) {
    const uint64_t now = SpickMonotonicNanoseconds();
    return now == 0 || now >= deadline;
}

static void SpickPruneLeases(void) {
    NSCAssert(NSThread.isMainThread, @"leases are owned by the IMK main thread");
    NSMutableDictionary *leases = SpickLeaseStore();
    const uint64_t now = SpickMonotonicNanoseconds();
    for (NSNumber *key in leases.allKeys) {
        SpickInputLease *lease = leases[key];
        if (lease.controller == nil || lease.expiresAtMonotonicNanoseconds <= now) {
            [leases removeObjectForKey:key];
        }
    }
    while (leases.count >= SpickMaximumLiveLeases) {
        NSNumber *key = leases.allKeys.firstObject;
        if (key == nil) {
            break;
        }
        [leases removeObjectForKey:key];
    }
}

static uint64_t SpickGenerateLeaseID(void) {
    NSMutableDictionary *leases = SpickLeaseStore();
    uint64_t leaseID = 0;
    do {
        arc4random_buf(&leaseID, sizeof(leaseID));
    } while (leaseID == 0 || leases[@(leaseID)] != nil);
    return leaseID;
}

static BOOL SpickWaitForDescriptor(int descriptor, short events, uint64_t deadline) {
    for (;;) {
        const uint64_t now = SpickMonotonicNanoseconds();
        if (now == 0 || now >= deadline) {
            errno = ETIMEDOUT;
            return NO;
        }
        const uint64_t remaining = deadline - now;
        const uint64_t roundedMilliseconds =
            remaining / NSEC_PER_MSEC + (remaining % NSEC_PER_MSEC != 0);
        const int timeout =
            (int)MIN(roundedMilliseconds, (uint64_t)INT_MAX);
        struct pollfd pollDescriptor = {
            .fd = descriptor,
            .events = events,
            .revents = 0,
        };
        const int result = poll(&pollDescriptor, 1, timeout);
        if (result > 0) {
            if ((pollDescriptor.revents & events) != 0) {
                if (!SpickDeadlinePassed(deadline)) {
                    return YES;
                }
                errno = ETIMEDOUT;
                return NO;
            }
            errno = ECONNRESET;
            return NO;
        }
        if (result == 0) {
            errno = ETIMEDOUT;
            return NO;
        }
        if (errno != EINTR) {
            return NO;
        }
    }
}

static BOOL SpickReadExactly(int descriptor,
                             void *buffer,
                             size_t length,
                             uint64_t deadline) {
    uint8_t *cursor = buffer;
    size_t remaining = length;
    while (remaining > 0) {
        if (!SpickWaitForDescriptor(descriptor, POLLIN, deadline)) {
            return NO;
        }
        const ssize_t count = read(descriptor, cursor, remaining);
        if (count == 0) {
            return NO;
        }
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                continue;
            }
            return NO;
        }
        if (SpickDeadlinePassed(deadline)) {
            errno = ETIMEDOUT;
            return NO;
        }
        cursor += count;
        remaining -= (size_t)count;
    }
    return YES;
}

static BOOL SpickWriteExactly(int descriptor,
                              const void *buffer,
                              size_t length,
                              uint64_t deadline) {
    const uint8_t *cursor = buffer;
    size_t remaining = length;
    while (remaining > 0) {
        if (!SpickWaitForDescriptor(descriptor, POLLOUT, deadline)) {
            return NO;
        }
        const ssize_t count = write(descriptor, cursor, remaining);
        if (count == 0) {
            errno = EPIPE;
            return NO;
        }
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                continue;
            }
            return NO;
        }
        if (SpickDeadlinePassed(deadline)) {
            errno = ETIMEDOUT;
            return NO;
        }
        cursor += count;
        remaining -= (size_t)count;
    }
    return YES;
}

static BOOL SpickPeerIsDesktopApplication(int descriptor) {
    uid_t peerUser = 0;
    gid_t peerGroup = 0;
    if (getpeereid(descriptor, &peerUser, &peerGroup) != 0 || peerUser != geteuid()) {
        return NO;
    }
    (void)peerGroup;

    pid_t peerProcess = 0;
    socklen_t peerProcessLength = sizeof(peerProcess);
    if (getsockopt(descriptor, SOL_LOCAL, LOCAL_PEERPID, &peerProcess,
                   &peerProcessLength) != 0 ||
        peerProcess <= 0) {
        return NO;
    }
    NSRunningApplication *application =
        [NSRunningApplication runningApplicationWithProcessIdentifier:peerProcess];
    return [application.bundleIdentifier isEqualToString:SpickDesktopBundleIdentifier];
}

static SpickInputResult SpickResult(SpickInsertStatus status, uint64_t leaseID) {
    return (SpickInputResult){.status = status, .leaseID = leaseID};
}

typedef NS_ENUM(int, SpickTransactionState) {
    SpickTransactionPending = 0,
    SpickTransactionClaimed = 1,
    SpickTransactionTimedOut = 2,
    SpickTransactionCompleted = 3,
};

static SpickInputResult SpickHandleRequest(SpickInputRequest *request,
                                            uint64_t monotonicDeadline);

static void SpickCancelTimedOutRequest(SpickInputRequest *request) {
    NSCAssert(NSThread.isMainThread, @"leases are owned by the IMK main thread");
    if (request.operation != SpickRequestOperationInsert &&
        request.operation != SpickRequestOperationDisarm) {
        return;
    }
    SpickInputLease *lease = SpickLeaseStore()[@(request.leaseID)];
    if (lease != nil && lease.requestID == request.requestID) {
        [SpickLeaseStore() removeObjectForKey:@(request.leaseID)];
    }
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
    if (!SpickPeerIsDesktopApplication(client)) {
        return;
    }
    const int enabled = 1;
    (void)setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof(enabled));
    const int descriptorFlags = fcntl(client, F_GETFD);
    const int statusFlags = fcntl(client, F_GETFL);
    if (descriptorFlags < 0 || statusFlags < 0 ||
        fcntl(client, F_SETFD, descriptorFlags | FD_CLOEXEC) != 0 ||
        fcntl(client, F_SETFL, statusFlags | O_NONBLOCK) != 0) {
        return;
    }

    const uint64_t acceptedAt = SpickMonotonicNanoseconds();
    if (acceptedAt == 0 ||
        acceptedAt > UINT64_MAX - SpickConnectionLifetimeNanoseconds) {
        return;
    }
    const uint64_t frameDeadline =
        acceptedAt + SpickConnectionLifetimeNanoseconds;

    NSMutableData *frame = [NSMutableData dataWithLength:SpickRequestHeaderLength];
    if (!SpickReadExactly(client, frame.mutableBytes, SpickRequestHeaderLength,
                          frameDeadline)) {
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
                          payloadLength, frameDeadline)) {
        return;
    }

    SpickInputRequest *request = SpickDecodeInputRequest(frame);
    if (request == nil) {
        NSData *response = SpickEncodeResponse(
            SpickResult(SpickInsertStatusInvalidRequest, 0), requestID);
        (void)SpickWriteExactly(client, response.bytes, response.length,
                                frameDeadline);
        return;
    }

    const uint64_t now = SpickCurrentEpochMilliseconds();
    if (request.expiresAtMilliseconds <= now ||
        request.expiresAtMilliseconds - now > 5000) {
        NSData *response = SpickEncodeResponse(
            SpickResult(SpickInsertStatusRequestExpired, 0), request.requestID);
        (void)SpickWriteExactly(client, response.bytes, response.length,
                                frameDeadline);
        return;
    }

    const uint64_t remainingMilliseconds = request.expiresAtMilliseconds - now;
    const uint64_t monotonicNow = SpickMonotonicNanoseconds();
    if (monotonicNow == 0 ||
        remainingMilliseconds > (UINT64_MAX - monotonicNow) / NSEC_PER_MSEC) {
        return;
    }
    const uint64_t requestDeadline =
        monotonicNow + remainingMilliseconds * NSEC_PER_MSEC;
    const uint64_t responseDeadline =
        requestDeadline > UINT64_MAX - (100 * NSEC_PER_MSEC)
            ? UINT64_MAX
            : requestDeadline + (100 * NSEC_PER_MSEC);

    __block SpickInputResult result = SpickResult(SpickInsertStatusInternalError, 0);
    __block atomic_int transactionState;
    atomic_init(&transactionState, SpickTransactionPending);
    dispatch_semaphore_t completed = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_main_queue(), ^{
      int expected = SpickTransactionPending;
      if (!atomic_compare_exchange_strong(&transactionState, &expected,
                                          SpickTransactionClaimed)) {
          SpickCancelTimedOutRequest(request);
          dispatch_semaphore_signal(completed);
          return;
      }

      result = SpickHandleRequest(request, requestDeadline);
      if (result.status == SpickInsertStatusArmed &&
          SpickDeadlinePassed(requestDeadline)) {
          [SpickLeaseStore() removeObjectForKey:@(result.leaseID)];
          result = SpickResult(SpickInsertStatusRequestExpired, 0);
      }

      expected = SpickTransactionClaimed;
      if (!atomic_compare_exchange_strong(&transactionState, &expected,
                                          SpickTransactionCompleted)) {
          SpickCancelTimedOutRequest(request);
          if (result.status == SpickInsertStatusArmed) {
              [SpickLeaseStore() removeObjectForKey:@(result.leaseID)];
              result = SpickResult(SpickInsertStatusRequestExpired, 0);
          }
      }
      dispatch_semaphore_signal(completed);
    });

    BOOL finished = NO;
    while (!SpickDeadlinePassed(requestDeadline)) {
        if (dispatch_semaphore_wait(completed, DISPATCH_TIME_NOW) == 0) {
            finished = YES;
            break;
        }
        const uint64_t currentTime = SpickMonotonicNanoseconds();
        const uint64_t remaining = requestDeadline > currentTime
                                       ? requestDeadline - currentTime
                                       : 0;
        const int64_t slice = (int64_t)MIN(remaining, 10 * NSEC_PER_MSEC);
        if (dispatch_semaphore_wait(
                completed, dispatch_time(DISPATCH_TIME_NOW, slice)) == 0) {
            finished = YES;
            break;
        }
    }
    SpickInputResult finalResult = SpickResult(SpickInsertStatusInternalError, 0);
    if (finished) {
        finalResult = result;
    }
    if (!finished) {
        int observed = atomic_load(&transactionState);
        while (observed == SpickTransactionPending ||
               observed == SpickTransactionClaimed) {
            const int previous = observed;
            if (atomic_compare_exchange_weak(&transactionState, &observed,
                                             SpickTransactionTimedOut)) {
                finalResult =
                    SpickResult(previous == SpickTransactionClaimed &&
                                        request.operation == SpickRequestOperationInsert
                                    ? SpickInsertStatusDispatched
                                    : SpickInsertStatusRequestExpired,
                                0);
                break;
            }
        }
        if (observed == SpickTransactionCompleted) {
            finalResult = result;
        } else if (observed == SpickTransactionTimedOut) {
            finalResult = SpickResult(SpickInsertStatusRequestExpired, 0);
        }
    }
    NSData *response = SpickEncodeResponse(finalResult, request.requestID);
    (void)SpickWriteExactly(client, response.bytes, response.length,
                            responseDeadline);
}

@end

void SpickStartInsertionBroker(void) {
    [[SpickInsertionBroker sharedBroker] start];
}

@implementation SpickInputController

- (void)activateServer:(id)sender {
    [super activateServer:sender];
    self.activationGeneration += 1;
    SpickActiveController = self;
}

- (void)deactivateServer:(id)sender {
    self.activationGeneration += 1;
    if (SpickActiveController == self) {
        SpickActiveController = nil;
    }
    [super deactivateServer:sender];
}

- (void)inputControllerWillClose {
    self.activationGeneration += 1;
    if (SpickActiveController == self) {
        SpickActiveController = nil;
    }
    [super inputControllerWillClose];
}

- (SpickInputResult)armRequest:(SpickInputRequest *)request
              monotonicDeadline:(uint64_t)monotonicDeadline {
    BOOL attempted = NO;
    @try {
        if (IsSecureEventInputEnabled()) {
            return SpickResult(SpickInsertStatusSecureInput, 0);
        }
        id<IMKTextInput, NSObject> client = self.client;
        if (client == nil) {
            return SpickResult(SpickInsertStatusNoActiveClient, 0);
        }
        if (![client supportsUnicode]) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        NSString *bundleIdentifier = [client bundleIdentifier];
        if (bundleIdentifier == nil ||
            ![bundleIdentifier isEqualToString:request.bundleIdentifier]) {
            return SpickResult(SpickInsertStatusTargetMismatch, 0);
        }
        NSRange selection = [client selectedRange];
        if (selection.location == NSNotFound || selection.length == NSNotFound) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        if (!NSEqualRanges(selection, request.selection)) {
            return SpickResult(SpickInsertStatusSelectionChanged, 0);
        }
        if ([client markedRange].location != NSNotFound) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        NSString *clientIdentifier = [client uniqueClientIdentifierString];
        if (clientIdentifier.length == 0) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        if (SpickDeadlinePassed(monotonicDeadline)) {
            return SpickResult(SpickInsertStatusRequestExpired, 0);
        }

        attempted = YES;
        SpickPruneLeases();
        const uint64_t monotonicNow = SpickMonotonicNanoseconds();
        if (monotonicNow == 0 ||
            monotonicNow > UINT64_MAX - SpickLeaseLifetimeNanoseconds) {
            return SpickResult(SpickInsertStatusInternalError, 0);
        }
        const uint64_t leaseID = SpickGenerateLeaseID();
        SpickInputLease *lease = [[SpickInputLease alloc] init];
        lease.leaseID = leaseID;
        lease.requestID = request.requestID;
        lease.expiresAtMonotonicNanoseconds =
            monotonicNow + SpickLeaseLifetimeNanoseconds;
        lease.activationGeneration = self.activationGeneration;
        lease.controller = self;
        lease.clientIdentifier = clientIdentifier;
        lease.bundleIdentifier = bundleIdentifier;
        lease.selection = selection;
        SpickLeaseStore()[@(leaseID)] = lease;
        return SpickResult(SpickInsertStatusArmed, leaseID);
    } @catch (__unused NSException *exception) {
        return SpickResult(attempted ? SpickInsertStatusInternalError
                                     : SpickInsertStatusUnsupported,
                           0);
    }
}

- (SpickInputResult)insertRequest:(SpickInputRequest *)request
                            lease:(SpickInputLease *)lease
                monotonicDeadline:(uint64_t)monotonicDeadline {
    BOOL attempted = NO;
    @try {
        if (lease.expiresAtMonotonicNanoseconds <= SpickMonotonicNanoseconds()) {
            return SpickResult(SpickInsertStatusLeaseExpired, 0);
        }
        if (lease.controller != self ||
            lease.activationGeneration != self.activationGeneration) {
            return SpickResult(SpickInsertStatusTargetMismatch, 0);
        }
        if (IsSecureEventInputEnabled()) {
            return SpickResult(SpickInsertStatusSecureInput, 0);
        }

        id<IMKTextInput, NSObject> client = self.client;
        if (client == nil || ![client supportsUnicode]) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        NSString *bundleIdentifier = [client bundleIdentifier];
        NSString *clientIdentifier = [client uniqueClientIdentifierString];
        if (![bundleIdentifier isEqualToString:lease.bundleIdentifier] ||
            ![bundleIdentifier isEqualToString:request.bundleIdentifier] ||
            ![clientIdentifier isEqualToString:lease.clientIdentifier]) {
            return SpickResult(SpickInsertStatusTargetMismatch, 0);
        }
        NSRange selection = [client selectedRange];
        if (!NSEqualRanges(selection, lease.selection) ||
            !NSEqualRanges(selection, request.selection)) {
            return SpickResult(SpickInsertStatusSelectionChanged, 0);
        }
        if ([client markedRange].location != NSNotFound) {
            return SpickResult(SpickInsertStatusUnsupported, 0);
        }
        if (request.text.length > NSUIntegerMax - selection.location) {
            return SpickResult(SpickInsertStatusInvalidRequest, 0);
        }
        if (SpickDeadlinePassed(monotonicDeadline)) {
            return SpickResult(SpickInsertStatusRequestExpired, 0);
        }

        attempted = YES;
        [client insertText:request.text
            replacementRange:NSMakeRange(NSNotFound, NSNotFound)];

        const NSRange insertedRange = NSMakeRange(selection.location, request.text.length);
        const NSRange expectedSelection = NSMakeRange(NSMaxRange(insertedRange), 0);
        const NSRange resultingSelection = [client selectedRange];
        NSRange actualRange = NSMakeRange(NSNotFound, 0);
        NSString *insertedText = [client stringFromRange:insertedRange actualRange:&actualRange];
        const BOOL confirmed = NSEqualRanges(resultingSelection, expectedSelection) &&
                               NSEqualRanges(actualRange, insertedRange) &&
                               [insertedText isEqualToString:request.text];
        return SpickResult(confirmed ? SpickInsertStatusConfirmed
                                     : SpickInsertStatusDispatched,
                           0);
    } @catch (__unused NSException *exception) {
        return SpickResult(attempted ? SpickInsertStatusDispatched
                                     : SpickInsertStatusInternalError,
                           0);
    }
}

@end

static SpickInputResult SpickHandleRequest(SpickInputRequest *request,
                                            uint64_t monotonicDeadline) {
    NSCAssert(NSThread.isMainThread, @"InputMethodKit calls must stay on the main thread");
    if (SpickDeadlinePassed(monotonicDeadline)) {
        return SpickResult(SpickInsertStatusRequestExpired, 0);
    }
    switch (request.operation) {
        case SpickRequestOperationArm: {
            SpickInputController *controller = SpickActiveController;
            return controller == nil
                       ? SpickResult(SpickInsertStatusNoActiveClient, 0)
                       : [controller armRequest:request
                                     monotonicDeadline:monotonicDeadline];
        }
        case SpickRequestOperationInsert: {
            SpickInputLease *lease = SpickLeaseStore()[@(request.leaseID)];
            [SpickLeaseStore() removeObjectForKey:@(request.leaseID)];
            if (lease == nil || lease.requestID != request.requestID) {
                return SpickResult(SpickInsertStatusLeaseMissingOrConsumed, 0);
            }
            SpickInputController *controller = SpickActiveController;
            if (controller == nil || controller != lease.controller) {
                return SpickResult(SpickInsertStatusTargetMismatch, 0);
            }
            return [controller insertRequest:request
                                        lease:lease
                            monotonicDeadline:monotonicDeadline];
        }
        case SpickRequestOperationDisarm: {
            SpickInputLease *lease = SpickLeaseStore()[@(request.leaseID)];
            if (lease != nil && lease.requestID == request.requestID) {
                [SpickLeaseStore() removeObjectForKey:@(request.leaseID)];
            }
            return SpickResult(SpickInsertStatusDisarmed, 0);
        }
    }
    return SpickResult(SpickInsertStatusInvalidRequest, 0);
}
