#import "SpickWireProtocol.h"

#include <limits.h>

const NSUInteger SpickRequestHeaderLength = 40;
const NSUInteger SpickResponseLength = 16;
const NSUInteger SpickMaximumBundleIdentifierBytes = 512;
const NSUInteger SpickMaximumTranscriptBytes = 1024 * 1024;

static const uint8_t SpickRequestMagic[] = {'S', 'P', 'K', '1'};
static const uint8_t SpickResponseMagic[] = {'S', 'P', 'R', '1'};
static const uint8_t SpickProtocolVersion = 1;
static const uint8_t SpickInsertOperation = 1;

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

@implementation SpickInsertRequest

- (instancetype)initWithRequestID:(uint64_t)requestID
                         selection:(NSRange)selection
                  bundleIdentifier:(NSString *)bundleIdentifier
                              text:(NSString *)text {
    self = [super init];
    if (self != nil) {
        _requestID = requestID;
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
        bytes[4] != SpickProtocolVersion || bytes[5] != SpickInsertOperation ||
        bytes[6] != 0 || bytes[7] != 0) {
        return NO;
    }

    const uint32_t bundleLength = SpickReadU32(bytes + 32);
    const uint32_t textLength = SpickReadU32(bytes + 36);
    if (bundleLength == 0 || bundleLength > SpickMaximumBundleIdentifierBytes ||
        textLength == 0 || textLength > SpickMaximumTranscriptBytes) {
        return NO;
    }

    const NSUInteger payloadLength = (NSUInteger)bundleLength + (NSUInteger)textLength;
    if (payloadLength > NSUIntegerMax - SpickRequestHeaderLength) {
        return NO;
    }

    *requestID = SpickReadU64(bytes + 8);
    *frameLength = SpickRequestHeaderLength + payloadLength;
    return YES;
}

SpickInsertRequest *SpickDecodeInsertRequest(NSData *frame) {
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
    const uint64_t location = SpickReadU64(bytes + 16);
    const uint64_t length = SpickReadU64(bytes + 24);
    if (location > NSUIntegerMax || length > NSUIntegerMax ||
        length > NSUIntegerMax - location) {
        return nil;
    }

    const uint32_t bundleLength = SpickReadU32(bytes + 32);
    const uint32_t textLength = SpickReadU32(bytes + 36);
    const uint8_t *payload = bytes + SpickRequestHeaderLength;
    NSData *bundleData = [NSData dataWithBytes:payload length:bundleLength];
    NSData *textData = [NSData dataWithBytes:payload + bundleLength length:textLength];
    NSString *bundleIdentifier = [[NSString alloc] initWithData:bundleData
                                                       encoding:NSUTF8StringEncoding];
    NSString *text = [[NSString alloc] initWithData:textData encoding:NSUTF8StringEncoding];
    if (bundleIdentifier.length == 0 || text.length == 0 ||
        [bundleIdentifier rangeOfCharacterFromSet:NSCharacterSet.controlCharacterSet].location !=
            NSNotFound ||
        text == nil) {
        return nil;
    }

    return [[SpickInsertRequest alloc]
        initWithRequestID:requestID
                 selection:NSMakeRange((NSUInteger)location, (NSUInteger)length)
          bundleIdentifier:bundleIdentifier
                      text:text];
}

NSData *SpickEncodeResponse(SpickInsertStatus status, uint64_t requestID) {
    NSMutableData *response = [NSMutableData dataWithCapacity:SpickResponseLength];
    [response appendBytes:SpickResponseMagic length:sizeof(SpickResponseMagic)];
    const uint8_t metadata[] = {SpickProtocolVersion, (uint8_t)status, 0, 0};
    [response appendBytes:metadata length:sizeof(metadata)];
    SpickAppendU64(response, requestID);
    return response;
}

static NSData *SpickEncodeRequestForTesting(uint64_t requestID,
                                            NSRange selection,
                                            NSString *bundleIdentifier,
                                            NSString *text) {
    NSData *bundleData = [bundleIdentifier dataUsingEncoding:NSUTF8StringEncoding];
    NSData *textData = [text dataUsingEncoding:NSUTF8StringEncoding];
    NSMutableData *frame = [NSMutableData dataWithCapacity:SpickRequestHeaderLength +
                                                          bundleData.length + textData.length];
    [frame appendBytes:SpickRequestMagic length:sizeof(SpickRequestMagic)];
    const uint8_t metadata[] = {SpickProtocolVersion, SpickInsertOperation, 0, 0};
    [frame appendBytes:metadata length:sizeof(metadata)];
    SpickAppendU64(frame, requestID);
    SpickAppendU64(frame, selection.location);
    SpickAppendU64(frame, selection.length);
    SpickAppendU32(frame, (uint32_t)bundleData.length);
    SpickAppendU32(frame, (uint32_t)textData.length);
    [frame appendData:bundleData];
    [frame appendData:textData];
    return frame;
}

BOOL SpickRunWireProtocolSelfTests(void) {
    NSString *sample = @"नमस्ते 👋 — مرحباً";
    NSData *frame = SpickEncodeRequestForTesting(42, NSMakeRange(12, 3),
                                                 @"com.example.Editor", sample);
    NSUInteger frameLength = 0;
    uint64_t requestID = 0;
    NSData *header = [frame subdataWithRange:NSMakeRange(0, SpickRequestHeaderLength)];
    if (!SpickRequestFrameLengthFromHeader(header, &frameLength, &requestID) ||
        frameLength != frame.length || requestID != 42) {
        return NO;
    }

    SpickInsertRequest *request = SpickDecodeInsertRequest(frame);
    if (request == nil || request.requestID != 42 ||
        !NSEqualRanges(request.selection, NSMakeRange(12, 3)) ||
        ![request.bundleIdentifier isEqualToString:@"com.example.Editor"] ||
        ![request.text isEqualToString:sample]) {
        return NO;
    }

    NSMutableData *badMagic = [frame mutableCopy];
    ((uint8_t *)badMagic.mutableBytes)[0] = 'X';
    if (SpickDecodeInsertRequest(badMagic) != nil) {
        return NO;
    }

    NSMutableData *oversized = [header mutableCopy];
    uint8_t *oversizedBytes = oversized.mutableBytes;
    const uint32_t tooLong = (uint32_t)SpickMaximumTranscriptBytes + 1;
    oversizedBytes[36] = (uint8_t)(tooLong >> 24);
    oversizedBytes[37] = (uint8_t)(tooLong >> 16);
    oversizedBytes[38] = (uint8_t)(tooLong >> 8);
    oversizedBytes[39] = (uint8_t)tooLong;
    if (SpickRequestFrameLengthFromHeader(oversized, &frameLength, &requestID)) {
        return NO;
    }

    NSData *response = SpickEncodeResponse(SpickInsertStatusConfirmed, 42);
    if (response.length != SpickResponseLength) {
        return NO;
    }
    return YES;
}
