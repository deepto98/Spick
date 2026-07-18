#import "SpickWireProtocol.h"

#include <limits.h>

const NSUInteger SpickRequestHeaderLength = 56;
const NSUInteger SpickResponseLength = 24;
const NSUInteger SpickMaximumBundleIdentifierBytes = 512;
const NSUInteger SpickMaximumTranscriptBytes = 1024 * 1024;

static const uint8_t SpickRequestMagic[] = {'S', 'P', 'K', '2'};
static const uint8_t SpickResponseMagic[] = {'S', 'P', 'R', '2'};
static const uint8_t SpickProtocolVersion = 2;

static uint32_t SpickReadU32(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
           ((uint32_t)bytes[2] << 8) | (uint32_t)bytes[3];
}

static uint64_t SpickReadU64(const uint8_t *bytes) {
    uint64_t value = 0;
    for (NSUInteger index = 0; index < 8; index += 1) {
        value = (value << 8) | bytes[index];
    }
    return value;
}

static void SpickAppendU32(NSMutableData *data, uint32_t value) {
    const uint8_t bytes[] = {
        (uint8_t)(value >> 24),
        (uint8_t)(value >> 16),
        (uint8_t)(value >> 8),
        (uint8_t)value,
    };
    [data appendBytes:bytes length:sizeof(bytes)];
}

static void SpickAppendU64(NSMutableData *data, uint64_t value) {
    uint8_t bytes[8];
    for (NSInteger index = 7; index >= 0; index -= 1) {
        bytes[index] = (uint8_t)value;
        value >>= 8;
    }
    [data appendBytes:bytes length:sizeof(bytes)];
}

static BOOL SpickOperationAndLengthsAreValid(SpickRequestOperation operation,
                                              uint32_t bundleLength,
                                              uint32_t textLength) {
    switch (operation) {
        case SpickRequestOperationArm:
            return bundleLength > 0 && bundleLength <= SpickMaximumBundleIdentifierBytes &&
                   textLength == 0;
        case SpickRequestOperationInsert:
            return bundleLength > 0 && bundleLength <= SpickMaximumBundleIdentifierBytes &&
                   textLength > 0 && textLength <= SpickMaximumTranscriptBytes;
        case SpickRequestOperationDisarm:
            return bundleLength == 0 && textLength == 0;
    }
    return NO;
}

@implementation SpickInputRequest

- (instancetype)initWithOperation:(SpickRequestOperation)operation
                         requestID:(uint64_t)requestID
                           leaseID:(uint64_t)leaseID
             expiresAtMilliseconds:(uint64_t)expiresAtMilliseconds
                         selection:(NSRange)selection
                  bundleIdentifier:(NSString *)bundleIdentifier
                              text:(NSString *)text {
    self = [super init];
    if (self != nil) {
        _operation = operation;
        _requestID = requestID;
        _leaseID = leaseID;
        _expiresAtMilliseconds = expiresAtMilliseconds;
        _selection = selection;
        _bundleIdentifier = [bundleIdentifier copy];
        _text = [text copy];
    }
    return self;
}

@end

BOOL SpickRequestFrameLengthFromHeader(NSData *header,
                                       NSUInteger *frameLength,
                                       uint64_t *requestID) {
    if (header.length != SpickRequestHeaderLength || frameLength == NULL ||
        requestID == NULL) {
        return NO;
    }

    const uint8_t *bytes = header.bytes;
    if (memcmp(bytes, SpickRequestMagic, sizeof(SpickRequestMagic)) != 0 ||
        bytes[4] != SpickProtocolVersion || bytes[6] != 0 || bytes[7] != 0) {
        return NO;
    }

    const SpickRequestOperation operation = (SpickRequestOperation)bytes[5];
    const uint32_t bundleLength = SpickReadU32(bytes + 48);
    const uint32_t textLength = SpickReadU32(bytes + 52);
    if (!SpickOperationAndLengthsAreValid(operation, bundleLength, textLength)) {
        return NO;
    }

    const NSUInteger payloadLength = (NSUInteger)bundleLength + (NSUInteger)textLength;
    if (payloadLength > NSUIntegerMax - SpickRequestHeaderLength) {
        return NO;
    }

    *requestID = SpickReadU64(bytes + 8);
    *frameLength = SpickRequestHeaderLength + payloadLength;
    return *requestID != 0;
}

SpickInputRequest *SpickDecodeInputRequest(NSData *frame) {
    if (frame.length < SpickRequestHeaderLength) {
        return nil;
    }

    NSData *header = [frame subdataWithRange:NSMakeRange(0, SpickRequestHeaderLength)];
    NSUInteger expectedLength = 0;
    uint64_t requestID = 0;
    if (!SpickRequestFrameLengthFromHeader(header, &expectedLength, &requestID) ||
        frame.length != expectedLength) {
        return nil;
    }

    const uint8_t *bytes = frame.bytes;
    const SpickRequestOperation operation = (SpickRequestOperation)bytes[5];
    const uint64_t leaseID = SpickReadU64(bytes + 16);
    const uint64_t expiresAtMilliseconds = SpickReadU64(bytes + 24);
    const uint64_t location = SpickReadU64(bytes + 32);
    const uint64_t length = SpickReadU64(bytes + 40);
    if (expiresAtMilliseconds == 0 || location > NSUIntegerMax || length > NSUIntegerMax ||
        location == NSNotFound || length == NSNotFound ||
        length > NSUIntegerMax - location) {
        return nil;
    }
    if ((operation == SpickRequestOperationArm && leaseID != 0) ||
        (operation != SpickRequestOperationArm && leaseID == 0)) {
        return nil;
    }

    const uint32_t bundleLength = SpickReadU32(bytes + 48);
    const uint32_t textLength = SpickReadU32(bytes + 52);
    const uint8_t *payload = bytes + SpickRequestHeaderLength;
    NSData *bundleData = [NSData dataWithBytes:payload length:bundleLength];
    NSData *textData = [NSData dataWithBytes:payload + bundleLength length:textLength];
    NSString *bundleIdentifier =
        bundleLength == 0
            ? @""
            : [[NSString alloc] initWithData:bundleData encoding:NSUTF8StringEncoding];
    NSString *text = textLength == 0
                         ? @""
                         : [[NSString alloc] initWithData:textData encoding:NSUTF8StringEncoding];
    if (bundleIdentifier == nil || text == nil ||
        (bundleLength > 0 && bundleIdentifier.length == 0) ||
        (textLength > 0 && text.length == 0) ||
        [bundleIdentifier rangeOfCharacterFromSet:NSCharacterSet.controlCharacterSet].location !=
            NSNotFound) {
        return nil;
    }
    if (operation == SpickRequestOperationDisarm &&
        (location != 0 || length != 0 || bundleIdentifier.length != 0 || text.length != 0)) {
        return nil;
    }

    return [[SpickInputRequest alloc]
        initWithOperation:operation
                 requestID:requestID
                   leaseID:leaseID
     expiresAtMilliseconds:expiresAtMilliseconds
                 selection:NSMakeRange((NSUInteger)location, (NSUInteger)length)
          bundleIdentifier:bundleIdentifier
                      text:text];
}

NSData *SpickEncodeResponse(SpickInputResult result, uint64_t requestID) {
    NSMutableData *response = [NSMutableData dataWithCapacity:SpickResponseLength];
    [response appendBytes:SpickResponseMagic length:sizeof(SpickResponseMagic)];
    const uint8_t metadata[] = {SpickProtocolVersion, (uint8_t)result.status, 0, 0};
    [response appendBytes:metadata length:sizeof(metadata)];
    SpickAppendU64(response, requestID);
    SpickAppendU64(response, result.leaseID);
    return response;
}

uint64_t SpickCurrentEpochMilliseconds(void) {
    return (uint64_t)(NSDate.date.timeIntervalSince1970 * 1000.0);
}

static NSData *SpickEncodeRequestForTesting(SpickRequestOperation operation,
                                            uint64_t requestID,
                                            uint64_t leaseID,
                                            uint64_t expiresAtMilliseconds,
                                            NSRange selection,
                                            NSString *bundleIdentifier,
                                            NSString *text) {
    NSData *bundleData = [bundleIdentifier dataUsingEncoding:NSUTF8StringEncoding];
    NSData *textData = [text dataUsingEncoding:NSUTF8StringEncoding];
    NSMutableData *frame = [NSMutableData dataWithCapacity:SpickRequestHeaderLength +
                                                          bundleData.length + textData.length];
    [frame appendBytes:SpickRequestMagic length:sizeof(SpickRequestMagic)];
    const uint8_t metadata[] = {SpickProtocolVersion, (uint8_t)operation, 0, 0};
    [frame appendBytes:metadata length:sizeof(metadata)];
    SpickAppendU64(frame, requestID);
    SpickAppendU64(frame, leaseID);
    SpickAppendU64(frame, expiresAtMilliseconds);
    SpickAppendU64(frame, selection.location);
    SpickAppendU64(frame, selection.length);
    SpickAppendU32(frame, (uint32_t)bundleData.length);
    SpickAppendU32(frame, (uint32_t)textData.length);
    [frame appendData:bundleData];
    [frame appendData:textData];
    return frame;
}

BOOL SpickRunWireProtocolSelfTests(void) {
    const uint64_t expiry = SpickCurrentEpochMilliseconds() + 1000;
    NSData *armFrame = SpickEncodeRequestForTesting(SpickRequestOperationArm, 42, 0, expiry,
                                                    NSMakeRange(12, 3),
                                                    @"com.example.Editor", @"");
    SpickInputRequest *arm = SpickDecodeInputRequest(armFrame);
    if (arm == nil || arm.operation != SpickRequestOperationArm || arm.requestID != 42 ||
        arm.leaseID != 0 || !NSEqualRanges(arm.selection, NSMakeRange(12, 3))) {
        return NO;
    }

    NSString *sample = @"नमस्ते 👋 — مرحباً";
    NSData *insertFrame = SpickEncodeRequestForTesting(
        SpickRequestOperationInsert, 43, 99, expiry, NSMakeRange(12, 3),
        @"com.example.Editor", sample);
    NSUInteger frameLength = 0;
    uint64_t requestID = 0;
    NSData *header =
        [insertFrame subdataWithRange:NSMakeRange(0, SpickRequestHeaderLength)];
    if (!SpickRequestFrameLengthFromHeader(header, &frameLength, &requestID) ||
        frameLength != insertFrame.length || requestID != 43) {
        return NO;
    }
    SpickInputRequest *insert = SpickDecodeInputRequest(insertFrame);
    if (insert == nil || insert.operation != SpickRequestOperationInsert ||
        insert.leaseID != 99 || ![insert.text isEqualToString:sample]) {
        return NO;
    }

    NSData *disarmFrame = SpickEncodeRequestForTesting(
        SpickRequestOperationDisarm, 44, 99, expiry, NSMakeRange(0, 0), @"", @"");
    SpickInputRequest *disarm = SpickDecodeInputRequest(disarmFrame);
    if (disarm == nil || disarm.operation != SpickRequestOperationDisarm) {
        return NO;
    }

    const uint8_t goldenBytes[] = {
        'S', 'P', 'K', '2', 2, 2, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 42,
        0, 0, 0, 0, 0, 0, 0, 99,
        0, 0, 0, 0, 0, 1, 0xE2, 0x40,
        0, 0, 0, 0, 0, 0, 0, 12,
        0, 0, 0, 0, 0, 0, 0, 3,
        0, 0, 0, 18, 0, 0, 0, 2,
        'c', 'o', 'm', '.', 'e', 'x', 'a', 'm', 'p', 'l', 'e', '.', 'E', 'd', 'i', 't', 'o', 'r',
        'H', 'i',
    };
    NSData *goldenFrame = SpickEncodeRequestForTesting(
        SpickRequestOperationInsert, 42, 99, 123456, NSMakeRange(12, 3),
        @"com.example.Editor", @"Hi");
    if (![goldenFrame isEqualToData:[NSData dataWithBytes:goldenBytes
                                                        length:sizeof(goldenBytes)]]) {
        return NO;
    }

    NSMutableData *badMagic = [insertFrame mutableCopy];
    ((uint8_t *)badMagic.mutableBytes)[0] = 'X';
    if (SpickDecodeInputRequest(badMagic) != nil) {
        return NO;
    }

    NSMutableArray<NSData *> *malformedFrames = [NSMutableArray array];
    [malformedFrames addObject:[insertFrame subdataWithRange:NSMakeRange(0,
                                                               insertFrame.length - 1)]];
    NSMutableData *extraByte = [insertFrame mutableCopy];
    const uint8_t zero = 0;
    [extraByte appendBytes:&zero length:1];
    [malformedFrames addObject:extraByte];

    const NSUInteger offsets[] = {5, 6};
    const uint8_t replacements[] = {99, 1};
    for (NSUInteger index = 0; index < sizeof(offsets) / sizeof(offsets[0]); index += 1) {
        NSMutableData *mutated = [insertFrame mutableCopy];
        ((uint8_t *)mutated.mutableBytes)[offsets[index]] = replacements[index];
        [malformedFrames addObject:mutated];
    }

    NSMutableData *zeroRequestID = [insertFrame mutableCopy];
    memset((uint8_t *)zeroRequestID.mutableBytes + 8, 0, 8);
    [malformedFrames addObject:zeroRequestID];

    NSMutableData *zeroInsertLease = [insertFrame mutableCopy];
    memset((uint8_t *)zeroInsertLease.mutableBytes + 16, 0, 8);
    [malformedFrames addObject:zeroInsertLease];

    NSMutableData *reusedArmLease = [armFrame mutableCopy];
    ((uint8_t *)reusedArmLease.mutableBytes)[23] = 1;
    [malformedFrames addObject:reusedArmLease];

    NSMutableData *zeroExpiry = [insertFrame mutableCopy];
    memset((uint8_t *)zeroExpiry.mutableBytes + 24, 0, 8);
    [malformedFrames addObject:zeroExpiry];

    NSMutableData *invalidUTF8 = [insertFrame mutableCopy];
    ((uint8_t *)invalidUTF8.mutableBytes)[SpickRequestHeaderLength] = 0xFF;
    [malformedFrames addObject:invalidUTF8];

    NSMutableData *notFoundRange = [insertFrame mutableCopy];
    memset((uint8_t *)notFoundRange.mutableBytes + 32, 0xFF, 8);
    [malformedFrames addObject:notFoundRange];

    NSMutableData *overflowRange = [insertFrame mutableCopy];
    uint8_t *overflowBytes = overflowRange.mutableBytes;
    memset(overflowBytes + 32, 0xFF, 8);
    overflowBytes[39] = 0xFE;
    memset(overflowBytes + 40, 0, 8);
    overflowBytes[47] = 2;
    [malformedFrames addObject:overflowRange];

    NSMutableData *badDisarm = [disarmFrame mutableCopy];
    ((uint8_t *)badDisarm.mutableBytes)[39] = 1;
    [malformedFrames addObject:badDisarm];

    for (NSData *malformed in malformedFrames) {
        if (SpickDecodeInputRequest(malformed) != nil) {
            return NO;
        }
    }

    NSMutableData *oversized = [header mutableCopy];
    uint8_t *oversizedBytes = oversized.mutableBytes;
    const uint32_t tooLong = (uint32_t)SpickMaximumTranscriptBytes + 1;
    oversizedBytes[52] = (uint8_t)(tooLong >> 24);
    oversizedBytes[53] = (uint8_t)(tooLong >> 16);
    oversizedBytes[54] = (uint8_t)(tooLong >> 8);
    oversizedBytes[55] = (uint8_t)tooLong;
    if (SpickRequestFrameLengthFromHeader(oversized, &frameLength, &requestID)) {
        return NO;
    }

    const SpickInputResult result = {.status = SpickInsertStatusArmed, .leaseID = 99};
    NSData *response = SpickEncodeResponse(result, 42);
    const uint8_t expectedResponse[] = {
        'S', 'P', 'R', '2', 2, 10, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 42,
        0, 0, 0, 0, 0, 0, 0, 99,
    };
    return [response isEqualToData:[NSData dataWithBytes:expectedResponse
                                                   length:sizeof(expectedResponse)]];
}
