#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef NS_ENUM(uint8_t, SpickRequestOperation) {
    SpickRequestOperationArm = 1,
    SpickRequestOperationInsert = 2,
    SpickRequestOperationDisarm = 3,
};

typedef NS_ENUM(uint8_t, SpickInsertStatus) {
    SpickInsertStatusConfirmed = 1,
    SpickInsertStatusDispatched = 2,
    SpickInsertStatusNoActiveClient = 3,
    SpickInsertStatusTargetMismatch = 4,
    SpickInsertStatusSelectionChanged = 5,
    SpickInsertStatusUnsupported = 6,
    SpickInsertStatusSecureInput = 7,
    SpickInsertStatusInvalidRequest = 8,
    SpickInsertStatusInternalError = 9,
    SpickInsertStatusArmed = 10,
    SpickInsertStatusDisarmed = 11,
    SpickInsertStatusLeaseExpired = 12,
    SpickInsertStatusRequestExpired = 13,
    SpickInsertStatusLeaseMissingOrConsumed = 14,
};

FOUNDATION_EXPORT const NSUInteger SpickRequestHeaderLength;
FOUNDATION_EXPORT const NSUInteger SpickResponseLength;
FOUNDATION_EXPORT const NSUInteger SpickMaximumBundleIdentifierBytes;
FOUNDATION_EXPORT const NSUInteger SpickMaximumTranscriptBytes;

@interface SpickInputRequest : NSObject

@property(nonatomic, readonly) SpickRequestOperation operation;
@property(nonatomic, readonly) uint64_t requestID;
@property(nonatomic, readonly) uint64_t leaseID;
@property(nonatomic, readonly) uint64_t expiresAtMilliseconds;
@property(nonatomic, readonly) NSRange selection;
@property(nonatomic, copy, readonly) NSString *bundleIdentifier;
@property(nonatomic, copy, readonly) NSString *text;

- (instancetype)initWithOperation:(SpickRequestOperation)operation
                         requestID:(uint64_t)requestID
                           leaseID:(uint64_t)leaseID
             expiresAtMilliseconds:(uint64_t)expiresAtMilliseconds
                         selection:(NSRange)selection
                  bundleIdentifier:(NSString *)bundleIdentifier
                              text:(NSString *)text NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

@end

typedef struct SpickInputResult {
    SpickInsertStatus status;
    uint64_t leaseID;
} SpickInputResult;

BOOL SpickRequestFrameLengthFromHeader(NSData *header,
                                       NSUInteger *frameLength,
                                       uint64_t *requestID);
SpickInputRequest *_Nullable SpickDecodeInputRequest(NSData *frame);
NSData *SpickEncodeResponse(SpickInputResult result, uint64_t requestID);
uint64_t SpickCurrentEpochMilliseconds(void);
BOOL SpickRunWireProtocolSelfTests(void);

NS_ASSUME_NONNULL_END
