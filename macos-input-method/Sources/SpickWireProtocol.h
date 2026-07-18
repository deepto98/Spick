#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

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
};

FOUNDATION_EXPORT const NSUInteger SpickRequestHeaderLength;
FOUNDATION_EXPORT const NSUInteger SpickResponseLength;
FOUNDATION_EXPORT const NSUInteger SpickMaximumBundleIdentifierBytes;
FOUNDATION_EXPORT const NSUInteger SpickMaximumTranscriptBytes;

@interface SpickInsertRequest : NSObject

@property(nonatomic, readonly) uint64_t requestID;
@property(nonatomic, readonly) NSRange selection;
@property(nonatomic, copy, readonly) NSString *bundleIdentifier;
@property(nonatomic, copy, readonly) NSString *text;

- (instancetype)initWithRequestID:(uint64_t)requestID
                         selection:(NSRange)selection
                  bundleIdentifier:(NSString *)bundleIdentifier
                              text:(NSString *)text NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

@end

BOOL SpickRequestFrameLengthFromHeader(NSData *header,
                                       NSUInteger *frameLength,
                                       uint64_t *requestID);
SpickInsertRequest *_Nullable SpickDecodeInsertRequest(NSData *frame);
NSData *SpickEncodeResponse(SpickInsertStatus status, uint64_t requestID);
BOOL SpickRunWireProtocolSelfTests(void);

NS_ASSUME_NONNULL_END
