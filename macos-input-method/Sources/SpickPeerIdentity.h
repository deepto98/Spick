#ifndef SPICK_PEER_IDENTITY_H
#define SPICK_PEER_IDENTITY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef uint32_t SpickPeerTrustResult;

enum {
    SpickPeerTrustSecure = 0,
    SpickPeerTrustUnsafeDevelopment = 1,
    SpickPeerTrustInvalidInput = 10,
    SpickPeerTrustUserMismatch = 11,
    SpickPeerTrustAuditTokenUnavailable = 12,
    SpickPeerTrustCodeUnavailable = 13,
    SpickPeerTrustSignatureInvalid = 14,
    SpickPeerTrustIdentityMismatch = 15,
    SpickPeerTrustTeamMismatch = 16,
    SpickPeerTrustAdHocDenied = 17,
};

// Authenticates the process on the other end of a connected AF_UNIX socket.
// Production builds accept only Apple-anchored code signed by the same team.
SpickPeerTrustResult SpickVerifyPeerSocket(int descriptor,
                                           const char *expectedSelfIdentifier,
                                           const char *expectedPeerIdentifier);

// The compatibility harness uses this variant to bind evidence to the live
// audit-token-resolved peer. The output is lowercase hex and never a path,
// certificate name, Team ID, or audit token.
SpickPeerTrustResult SpickVerifyPeerSocketWithCDHash(
    int descriptor,
    const char *expectedSelfIdentifier,
    const char *expectedPeerIdentifier,
    char *peerCDHashHex,
    size_t peerCDHashHexCapacity);

// True only in artifacts compiled with the explicit unsafe development escape hatch.
bool SpickPeerAuthenticationAllowsUnsafeDevelopment(void);

// Pure policy tests used by the native build check. This does not open a broker.
bool SpickRunPeerIdentitySelfTests(void);

// Exercises audit-token resolution and the live signature policy against this process.
bool SpickRunPeerIdentityRuntimeSelfTest(const char *expectedSelfIdentifier);

#endif
