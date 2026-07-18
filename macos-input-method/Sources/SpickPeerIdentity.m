#import "SpickPeerIdentity.h"

#import <Foundation/Foundation.h>
#import <Security/Security.h>

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#ifndef SPICK_ALLOW_UNSAFE_ADHOC_PEERS
#define SPICK_ALLOW_UNSAFE_ADHOC_PEERS 0
#endif

@interface SpickCodeFacts : NSObject
@property(nonatomic, copy) NSString *identifier;
@property(nonatomic, copy) NSString *teamIdentifier;
@property(nonatomic, copy) NSDictionary<NSString *, id> *entitlements;
@property(nonatomic) SecCodeSignatureFlags flags;
@end

@implementation SpickCodeFacts
@end

static BOOL SpickIdentityComponentIsSafe(NSString *value, BOOL teamIdentifier) {
    if (value.length == 0 || value.length > 255) {
        return NO;
    }
    NSCharacterSet *allowed = teamIdentifier
                                  ? NSCharacterSet.alphanumericCharacterSet
                                  : [NSCharacterSet characterSetWithCharactersInString:
                                                        @"abcdefghijklmnopqrstuvwxyz"
                                                         "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                                                         "0123456789.-_"];
    return [value rangeOfCharacterFromSet:allowed.invertedSet].location == NSNotFound;
}

static NSString *SpickStringFromUTF8(const char *value) {
    if (value == NULL) {
        return nil;
    }
    NSString *string = [NSString stringWithUTF8String:value];
    return SpickIdentityComponentIsSafe(string, NO) ? string : nil;
}

static SpickCodeFacts *SpickCopyCodeFacts(SecCodeRef code) {
    CFDictionaryRef rawInformation = NULL;
    if (SecCodeCopySigningInformation(code, kSecCSSigningInformation,
                                      &rawInformation) != errSecSuccess ||
        rawInformation == NULL) {
        return nil;
    }
    NSDictionary *information = CFBridgingRelease(rawInformation);
    id identifier = information[(__bridge NSString *)kSecCodeInfoIdentifier];
    id teamIdentifier = information[(__bridge NSString *)kSecCodeInfoTeamIdentifier];
    id entitlements = information[(__bridge NSString *)kSecCodeInfoEntitlementsDict];
    id flags = information[(__bridge NSString *)kSecCodeInfoFlags];
    if (![identifier isKindOfClass:NSString.class] ||
        ![flags isKindOfClass:NSNumber.class] ||
        (entitlements != nil && ![entitlements isKindOfClass:NSDictionary.class])) {
        return nil;
    }

    SpickCodeFacts *facts = [[SpickCodeFacts alloc] init];
    facts.identifier = identifier;
    facts.teamIdentifier = [teamIdentifier isKindOfClass:NSString.class]
                               ? teamIdentifier
                               : @"";
    facts.entitlements = entitlements ?: @{};
    facts.flags = (SecCodeSignatureFlags)[flags unsignedIntValue];
    return facts;
}

static BOOL SpickCodeHasDangerousEntitlements(SpickCodeFacts *facts) {
    static NSArray<NSString *> *dangerousEntitlements;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
      dangerousEntitlements = @[
          @"com.apple.security.get-task-allow",
          @"com.apple.security.cs.disable-library-validation",
          @"com.apple.security.cs.allow-dyld-environment-variables",
          @"com.apple.security.cs.disable-executable-page-protection",
          @"com.apple.security.cs.allow-unsigned-executable-memory",
      ];
    });
    for (NSString *key in dangerousEntitlements) {
        if ([facts.entitlements[key] isEqual:@YES]) {
            return YES;
        }
    }
    return NO;
}

static SecRequirementRef SpickCreateRequirement(NSString *identifier,
                                                 NSString *teamIdentifier) {
    if (!SpickIdentityComponentIsSafe(identifier, NO) ||
        !SpickIdentityComponentIsSafe(teamIdentifier, YES)) {
        return NULL;
    }
    NSString *source = [NSString
        stringWithFormat:@"identifier \"%@\" and anchor apple generic and "
                          "certificate leaf[subject.OU] = \"%@\"",
                         identifier, teamIdentifier];
    SecRequirementRef requirement = NULL;
    CFErrorRef error = NULL;
    OSStatus status = SecRequirementCreateWithStringAndErrors(
        (__bridge CFStringRef)source, kSecCSDefaultFlags, &error, &requirement);
    if (error != NULL) {
        CFRelease(error);
    }
    return status == errSecSuccess ? requirement : NULL;
}

static BOOL SpickCodeSatisfiesRequirement(SecCodeRef code,
                                          SecRequirementRef requirement) {
    CFErrorRef error = NULL;
    const OSStatus status = SecCodeCheckValidityWithErrors(
        code, kSecCSNoNetworkAccess, requirement, &error);
    if (error != NULL) {
        CFRelease(error);
    }
    return status == errSecSuccess;
}

static BOOL SpickSecureFactsAccept(SpickCodeFacts *selfFacts,
                                   SpickCodeFacts *peerFacts,
                                   NSString *expectedSelfIdentifier,
                                   NSString *expectedPeerIdentifier) {
    if (selfFacts == nil || peerFacts == nil ||
        ![selfFacts.identifier isEqualToString:expectedSelfIdentifier] ||
        ![peerFacts.identifier isEqualToString:expectedPeerIdentifier]) {
        return NO;
    }
    if ((selfFacts.flags & kSecCodeSignatureAdhoc) != 0 ||
        (peerFacts.flags & kSecCodeSignatureAdhoc) != 0) {
        return NO;
    }
    if ((selfFacts.flags & kSecCodeSignatureRuntime) == 0 ||
        (peerFacts.flags & kSecCodeSignatureRuntime) == 0 ||
        SpickCodeHasDangerousEntitlements(selfFacts) ||
        SpickCodeHasDangerousEntitlements(peerFacts)) {
        return NO;
    }
    return SpickIdentityComponentIsSafe(selfFacts.teamIdentifier, YES) &&
           [selfFacts.teamIdentifier isEqualToString:peerFacts.teamIdentifier];
}

static SpickPeerTrustResult SpickVerifySecurePair(
    SecCodeRef selfCode,
    SecCodeRef peerCode,
    NSString *expectedSelfIdentifier,
    NSString *expectedPeerIdentifier) {
    // These first facts are untrusted. They are used only to construct the
    // Apple-anchored requirement that subsequently authenticates them.
    SpickCodeFacts *candidateSelfFacts = SpickCopyCodeFacts(selfCode);
    SpickCodeFacts *candidatePeerFacts = SpickCopyCodeFacts(peerCode);
    if (candidateSelfFacts == nil || candidatePeerFacts == nil) {
        return SpickPeerTrustSignatureInvalid;
    }
    if (![candidateSelfFacts.identifier isEqualToString:expectedSelfIdentifier] ||
        ![candidatePeerFacts.identifier isEqualToString:expectedPeerIdentifier]) {
        return SpickPeerTrustIdentityMismatch;
    }
    if ((candidateSelfFacts.flags & kSecCodeSignatureAdhoc) != 0 ||
        (candidatePeerFacts.flags & kSecCodeSignatureAdhoc) != 0) {
        return SpickPeerTrustAdHocDenied;
    }
    if ((candidateSelfFacts.flags & kSecCodeSignatureRuntime) == 0 ||
        (candidatePeerFacts.flags & kSecCodeSignatureRuntime) == 0 ||
        SpickCodeHasDangerousEntitlements(candidateSelfFacts) ||
        SpickCodeHasDangerousEntitlements(candidatePeerFacts)) {
        return SpickPeerTrustSignatureInvalid;
    }
    if (!SpickIdentityComponentIsSafe(candidateSelfFacts.teamIdentifier, YES) ||
        ![candidateSelfFacts.teamIdentifier
            isEqualToString:candidatePeerFacts.teamIdentifier]) {
        return SpickPeerTrustTeamMismatch;
    }

    SecRequirementRef selfRequirement = SpickCreateRequirement(
        expectedSelfIdentifier, candidateSelfFacts.teamIdentifier);
    SecRequirementRef peerRequirement = SpickCreateRequirement(
        expectedPeerIdentifier, candidateSelfFacts.teamIdentifier);
    if (selfRequirement == NULL || peerRequirement == NULL) {
        if (selfRequirement != NULL) {
            CFRelease(selfRequirement);
        }
        if (peerRequirement != NULL) {
            CFRelease(peerRequirement);
        }
        return SpickPeerTrustSignatureInvalid;
    }

    const BOOL requirementsPassed =
        SpickCodeSatisfiesRequirement(selfCode, selfRequirement) &&
        SpickCodeSatisfiesRequirement(peerCode, peerRequirement);
    CFRelease(selfRequirement);
    CFRelease(peerRequirement);
    if (!requirementsPassed) {
        return SpickPeerTrustSignatureInvalid;
    }

    // Re-read only after dynamic validation, as required by SecCode's contract.
    SpickCodeFacts *verifiedSelfFacts = SpickCopyCodeFacts(selfCode);
    SpickCodeFacts *verifiedPeerFacts = SpickCopyCodeFacts(peerCode);
    return SpickSecureFactsAccept(verifiedSelfFacts, verifiedPeerFacts,
                                  expectedSelfIdentifier, expectedPeerIdentifier)
               ? SpickPeerTrustSecure
               : SpickPeerTrustIdentityMismatch;
}

#if SPICK_ALLOW_UNSAFE_ADHOC_PEERS
static BOOL SpickCodeIsValidAdHoc(SecCodeRef code) {
    if (!SpickCodeSatisfiesRequirement(code, NULL)) {
        return NO;
    }
    SpickCodeFacts *facts = SpickCopyCodeFacts(code);
    return facts != nil && (facts.flags & kSecCodeSignatureAdhoc) != 0;
}

static BOOL SpickUnsafeDevelopmentPairAccepts(
    SecCodeRef selfCode,
    SecCodeRef peerCode,
    NSString *expectedSelfIdentifier,
    NSString *expectedPeerIdentifier) {
    if (!SpickCodeIsValidAdHoc(selfCode) || !SpickCodeIsValidAdHoc(peerCode)) {
        return NO;
    }
    SpickCodeFacts *selfFacts = SpickCopyCodeFacts(selfCode);
    SpickCodeFacts *peerFacts = SpickCopyCodeFacts(peerCode);
    return [selfFacts.identifier isEqualToString:expectedSelfIdentifier] &&
           [peerFacts.identifier isEqualToString:expectedPeerIdentifier];
}
#endif

SpickPeerTrustResult SpickVerifyPeerSocket(int descriptor,
                                           const char *expectedSelfIdentifier,
                                           const char *expectedPeerIdentifier) {
    @autoreleasepool {
        NSString *selfIdentifier = SpickStringFromUTF8(expectedSelfIdentifier);
        NSString *peerIdentifier = SpickStringFromUTF8(expectedPeerIdentifier);
        if (descriptor < 0 || selfIdentifier == nil || peerIdentifier == nil) {
            return SpickPeerTrustInvalidInput;
        }

        uid_t peerUser = 0;
        gid_t peerGroup = 0;
        if (getpeereid(descriptor, &peerUser, &peerGroup) != 0 ||
            peerUser != geteuid()) {
            return SpickPeerTrustUserMismatch;
        }
        (void)peerGroup;

        audit_token_t peerToken = INVALID_AUDIT_TOKEN_VALUE;
        socklen_t tokenLength = sizeof(peerToken);
        if (getsockopt(descriptor, SOL_LOCAL, LOCAL_PEERTOKEN, &peerToken,
                       &tokenLength) != 0 ||
            tokenLength != sizeof(peerToken)) {
            return SpickPeerTrustAuditTokenUnavailable;
        }

        NSData *auditData = [NSData dataWithBytes:&peerToken length:sizeof(peerToken)];
        NSDictionary *attributes = @{
            (__bridge NSString *)kSecGuestAttributeAudit : auditData,
        };
        SecCodeRef peerCode = NULL;
        SecCodeRef selfCode = NULL;
        if (SecCodeCopyGuestWithAttributes(NULL, (__bridge CFDictionaryRef)attributes,
                                           kSecCSDefaultFlags,
                                           &peerCode) != errSecSuccess ||
            peerCode == NULL ||
            SecCodeCopySelf(kSecCSDefaultFlags, &selfCode) != errSecSuccess ||
            selfCode == NULL) {
            if (peerCode != NULL) {
                CFRelease(peerCode);
            }
            if (selfCode != NULL) {
                CFRelease(selfCode);
            }
            return SpickPeerTrustCodeUnavailable;
        }

        SpickPeerTrustResult result = SpickVerifySecurePair(
            selfCode, peerCode, selfIdentifier, peerIdentifier);
#if SPICK_ALLOW_UNSAFE_ADHOC_PEERS
        if (result != SpickPeerTrustSecure &&
            SpickUnsafeDevelopmentPairAccepts(selfCode, peerCode, selfIdentifier,
                                              peerIdentifier)) {
            result = SpickPeerTrustUnsafeDevelopment;
        }
#endif
        CFRelease(selfCode);
        CFRelease(peerCode);
        return result;
    }
}

bool SpickPeerAuthenticationAllowsUnsafeDevelopment(void) {
    return SPICK_ALLOW_UNSAFE_ADHOC_PEERS != 0;
}

bool SpickRunPeerIdentitySelfTests(void) {
    @autoreleasepool {
        SpickCodeFacts *desktop = [[SpickCodeFacts alloc] init];
        desktop.identifier = @"app.spick.desktop";
        desktop.teamIdentifier = @"A1B2C3D4E5";
        desktop.entitlements = @{};
        desktop.flags = kSecCodeSignatureRuntime;
        SpickCodeFacts *helper = [[SpickCodeFacts alloc] init];
        helper.identifier = @"app.spick.desktop.input-method";
        helper.teamIdentifier = @"A1B2C3D4E5";
        helper.entitlements = @{};
        helper.flags = kSecCodeSignatureRuntime;
        if (!SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                    helper.identifier)) {
            return false;
        }

        helper.identifier = @"app.spick.desktop.input-method-copy";
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   @"app.spick.desktop.input-method")) {
            return false;
        }
        helper.identifier = @"app.spick.desktop.input-method";
        helper.teamIdentifier = @"Z9Y8X7W6V5";
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   helper.identifier)) {
            return false;
        }
        helper.teamIdentifier = desktop.teamIdentifier;
        helper.flags = kSecCodeSignatureAdhoc | kSecCodeSignatureRuntime;
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   helper.identifier)) {
            return false;
        }
        desktop.flags = kSecCodeSignatureAdhoc | kSecCodeSignatureRuntime;
        helper.flags = kSecCodeSignatureRuntime;
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   helper.identifier)) {
            return false;
        }
        desktop.flags = kSecCodeSignatureRuntime;
        helper.entitlements = @{ @"com.apple.security.get-task-allow" : @YES };
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   helper.identifier)) {
            return false;
        }
        helper.entitlements = @{};
        helper.flags = 0;
        if (SpickSecureFactsAccept(desktop, helper, desktop.identifier,
                                   helper.identifier)) {
            return false;
        }
        return !SpickIdentityComponentIsSafe(@"TEAM\" or true", YES) &&
               !SpickIdentityComponentIsSafe(@"bad identifier!", NO) &&
               SpickStringFromUTF8("app.spick.desktop") != nil &&
               SpickStringFromUTF8("app.spick.desktop\n") == nil;
    }
}

bool SpickRunPeerIdentityRuntimeSelfTest(const char *expectedSelfIdentifier) {
    NSString *identifier = SpickStringFromUTF8(expectedSelfIdentifier);
    SecCodeRef selfCode = NULL;
    if (identifier == nil ||
        SecCodeCopySelf(kSecCSDefaultFlags, &selfCode) != errSecSuccess ||
        selfCode == NULL) {
        return false;
    }
    SpickCodeFacts *selfFacts = SpickCopyCodeFacts(selfCode);
    CFRelease(selfCode);
    if (selfFacts == nil || ![selfFacts.identifier isEqualToString:identifier]) {
        return false;
    }

    int sockets[2] = {-1, -1};
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) != 0) {
        return false;
    }
    const SpickPeerTrustResult result = SpickVerifyPeerSocket(
        sockets[0], expectedSelfIdentifier, expectedSelfIdentifier);
    NSString *wrongIdentifier = [identifier stringByAppendingString:@".wrong"];
    const SpickPeerTrustResult wrongPeer = SpickVerifyPeerSocket(
        sockets[0], expectedSelfIdentifier, wrongIdentifier.UTF8String);
    const SpickPeerTrustResult wrongSelf = SpickVerifyPeerSocket(
        sockets[0], wrongIdentifier.UTF8String, expectedSelfIdentifier);
    close(sockets[0]);
    close(sockets[1]);
    if (wrongPeer != SpickPeerTrustIdentityMismatch ||
        wrongSelf != SpickPeerTrustIdentityMismatch) {
        return false;
    }

    if ((selfFacts.flags & kSecCodeSignatureAdhoc) == 0) {
        return result == SpickPeerTrustSecure;
    }
    if (SpickPeerAuthenticationAllowsUnsafeDevelopment()) {
        return result == SpickPeerTrustUnsafeDevelopment;
    }
    return result == SpickPeerTrustAdHocDenied;
}
