#import "SpickInputSourceInspection.h"

#import <Carbon/Carbon.h>
#import <Foundation/Foundation.h>

static NSString *SpickInspectionIdentifier(const char *value) {
    if (value == NULL) {
        return nil;
    }
    NSString *identifier = [NSString stringWithUTF8String:value];
    if (identifier.length == 0 || identifier.length > 255) {
        return nil;
    }
    NSCharacterSet *allowed =
        [NSCharacterSet characterSetWithCharactersInString:
                            @"abcdefghijklmnopqrstuvwxyz"
                             "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
                             "0123456789.-_"];
    return [identifier rangeOfCharacterFromSet:allowed.invertedSet].location ==
                   NSNotFound
               ? identifier
               : nil;
}

static BOOL SpickInspectionBoolean(TISInputSourceRef source, CFStringRef key) {
    CFTypeRef value = TISGetInputSourceProperty(source, key);
    return value != NULL && CFGetTypeID(value) == CFBooleanGetTypeID() &&
           CFBooleanGetValue((CFBooleanRef)value);
}

static BOOL SpickInspectionIdentityMatches(TISInputSourceRef source,
                                           NSString *identifier) {
    CFTypeRef sourceIdentifier =
        TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
    CFTypeRef category =
        TISGetInputSourceProperty(source, kTISPropertyInputSourceCategory);
    CFTypeRef bundleIdentifier =
        TISGetInputSourceProperty(source, kTISPropertyBundleID);
    return sourceIdentifier != NULL &&
           CFEqual(sourceIdentifier, (__bridge CFStringRef)identifier) &&
           category != NULL && CFEqual(category, kTISCategoryPaletteInputSource) &&
           bundleIdentifier != NULL &&
           CFEqual(bundleIdentifier, (__bridge CFStringRef)identifier) &&
           SpickInspectionBoolean(source,
                                  kTISPropertyInputSourceIsEnableCapable) &&
           SpickInspectionBoolean(source,
                                  kTISPropertyInputSourceIsSelectCapable);
}

SpickInputSourceState SpickInspectInputSourceState(
    const char *expectedIdentifier) {
    @autoreleasepool {
        NSString *identifier = SpickInspectionIdentifier(expectedIdentifier);
        if (identifier == nil) {
            return SpickInputSourceInvalid;
        }
        NSDictionary *properties = @{
            (__bridge NSString *)kTISPropertyInputSourceID : identifier,
        };
        CFArrayRef rawSources = TISCreateInputSourceList(
            (__bridge CFDictionaryRef)properties, true);
        NSArray *sources = CFBridgingRelease(rawSources) ?: @[];
        if (sources.count == 0) {
            return SpickInputSourceMissing;
        }
        if (sources.count != 1) {
            return SpickInputSourceInvalid;
        }

        TISInputSourceRef source =
            (__bridge TISInputSourceRef)sources.firstObject;
        if (!SpickInspectionIdentityMatches(source, identifier)) {
            return SpickInputSourceInvalid;
        }
        const BOOL selected = SpickInspectionBoolean(
            source, kTISPropertyInputSourceIsSelected);
        const BOOL enabled = SpickInspectionBoolean(
            source, kTISPropertyInputSourceIsEnabled);
        if (selected && !enabled) {
            return SpickInputSourceInvalid;
        }
        return selected ? SpickInputSourceSelected
                        : (enabled ? SpickInputSourceEnabled
                                   : SpickInputSourceDisabled);
    }
}
