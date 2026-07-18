#ifndef SPICK_INPUT_SOURCE_INSPECTION_H
#define SPICK_INPUT_SOURCE_INSPECTION_H

#include <stdint.h>

typedef uint32_t SpickInputSourceState;

enum {
    SpickInputSourceMissing = 0,
    SpickInputSourceDisabled = 1,
    SpickInputSourceEnabled = 2,
    SpickInputSourceSelected = 3,
    SpickInputSourceInvalid = 10,
};

// Read-only TIS inspection. This function never registers, enables, disables,
// selects, or deselects an input source.
SpickInputSourceState SpickInspectInputSourceState(
    const char *expectedIdentifier);

#endif
