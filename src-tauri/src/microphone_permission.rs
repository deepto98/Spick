//! Microphone authorization without touching the audio device.
//!
//! Dictation checks this module before capturing another application's text
//! target. In particular, `status` never prompts. The only prompt-capable API
//! is `request_access`, which is intended to be called from an explicit action
//! in Spick's main window.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MicrophonePermissionState {
    Granted,
    Missing,
    Restricted,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MicrophonePermissionStatus {
    pub state: MicrophonePermissionState,
    pub can_request: bool,
}

impl MicrophonePermissionStatus {
    const fn new(state: MicrophonePermissionState, can_request: bool) -> Self {
        Self { state, can_request }
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    const fn unsupported() -> Self {
        Self::new(MicrophonePermissionState::Unsupported, false)
    }
}

/// Read the current authorization state. This function never presents a
/// system dialog or opens an audio device.
pub(crate) fn status() -> Result<MicrophonePermissionStatus, String> {
    platform::status()
}

/// Begin Apple's microphone request when the user has not answered it before.
///
/// The caller must initiate this from a deliberate main-window action. Apple
/// may invoke `completion` on any dispatch queue, so the bound deliberately
/// requires a thread-safe callback. Already-decided and unsupported states are
/// returned through the callback immediately without invoking the prompt API.
pub(crate) fn request_access<F>(completion: F) -> Result<(), String>
where
    F: Fn(Result<MicrophonePermissionStatus, String>) + Send + Sync + 'static,
{
    let current = status()?;
    if !should_request_access(current) {
        completion(Ok(current));
        return Ok(());
    }

    platform::request_access(completion)
}

fn should_request_access(permission: MicrophonePermissionStatus) -> bool {
    permission.state == MicrophonePermissionState::Missing && permission.can_request
}

/// Reject capture before it can open a device or disturb another app's focus.
/// Unsupported platforms keep their existing behavior and let the audio
/// backend report platform-specific failures.
pub(crate) fn ensure_capture_allowed() -> Result<(), String> {
    validate_capture_status(status()?)
}

/// Open macOS System Settings at Privacy & Security > Microphone.
///
/// AppKit window operations are expected on the main thread; the command layer
/// should schedule this helper with `AppHandle::run_on_main_thread`.
pub(crate) fn open_microphone_privacy_settings() -> Result<(), String> {
    platform::open_microphone_privacy_settings()
}

fn validate_capture_status(permission: MicrophonePermissionStatus) -> Result<(), String> {
    match (permission.state, permission.can_request) {
        (MicrophonePermissionState::Granted, _)
        | (MicrophonePermissionState::Unsupported, _) => Ok(()),
        (MicrophonePermissionState::Missing, true) => Err(
            "Microphone access hasn’t been allowed yet. Open Spick and choose Allow microphone before dictating."
                .into(),
        ),
        (MicrophonePermissionState::Missing, false) => Err(
            "Microphone access is off. Turn on Spick in System Settings → Privacy & Security → Microphone, then try again."
                .into(),
        ),
        (MicrophonePermissionState::Restricted, _) => {
            Err("Microphone access is restricted by this Mac.".into())
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2_app_kit::NSWorkspace;
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio,
    };
    use objc2_foundation::{NSString, NSURL};

    use super::{MicrophonePermissionState, MicrophonePermissionStatus};

    // This deep link is accepted by System Settings on Spick's minimum macOS
    // version and avoids asking a denied user to hunt through the settings UI.
    const MICROPHONE_PRIVACY_SETTINGS_URL: &str =
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

    pub(super) fn status() -> Result<MicrophonePermissionStatus, String> {
        let media_type = audio_media_type()?;
        // SAFETY: AVMediaTypeAudio is one of the two values accepted by this
        // public class method. It is available before Spick's macOS 13 floor.
        let authorization = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
        map_authorization_status(authorization)
    }

    pub(super) fn request_access<F>(completion: F) -> Result<(), String>
    where
        F: Fn(Result<MicrophonePermissionStatus, String>) + Send + Sync + 'static,
    {
        let media_type = audio_media_type()?;
        let handler = RcBlock::new(move |_granted| completion(status()));
        // SAFETY: The media type is AVMediaTypeAudio and the escaping heap
        // block captures only a Send + Sync callback. Apple's API may invoke
        // it on an arbitrary dispatch queue, which the public bound permits.
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &handler)
        };
        Ok(())
    }

    pub(super) fn open_microphone_privacy_settings() -> Result<(), String> {
        let url_string = NSString::from_str(MICROPHONE_PRIVACY_SETTINGS_URL);
        let url = NSURL::URLWithString(&url_string)
            .ok_or_else(|| "could not construct the macOS Microphone settings URL".to_string())?;
        if NSWorkspace::sharedWorkspace().openURL(&url) {
            Ok(())
        } else {
            Err("macOS could not open Privacy & Security → Microphone".into())
        }
    }

    fn audio_media_type() -> Result<&'static AVMediaType, String> {
        // SAFETY: This reads the immutable AVFoundation constant. The symbol
        // has been present since macOS 10.7, before Spick's deployment floor.
        unsafe { AVMediaTypeAudio }
            .ok_or_else(|| "AVFoundation does not expose its audio media type".to_string())
    }

    fn map_authorization_status(
        authorization: AVAuthorizationStatus,
    ) -> Result<MicrophonePermissionStatus, String> {
        match authorization {
            AVAuthorizationStatus::NotDetermined => Ok(MicrophonePermissionStatus::new(
                MicrophonePermissionState::Missing,
                true,
            )),
            AVAuthorizationStatus::Denied => Ok(MicrophonePermissionStatus::new(
                MicrophonePermissionState::Missing,
                false,
            )),
            AVAuthorizationStatus::Restricted => Ok(MicrophonePermissionStatus::new(
                MicrophonePermissionState::Restricted,
                false,
            )),
            AVAuthorizationStatus::Authorized => Ok(MicrophonePermissionStatus::new(
                MicrophonePermissionState::Granted,
                false,
            )),
            unknown => Err(format!(
                "macOS returned an unknown microphone authorization status ({})",
                unknown.0
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn maps_every_documented_avfoundation_authorization_status() {
            assert_eq!(
                map_authorization_status(AVAuthorizationStatus::NotDetermined).unwrap(),
                MicrophonePermissionStatus::new(MicrophonePermissionState::Missing, true)
            );
            assert_eq!(
                map_authorization_status(AVAuthorizationStatus::Denied).unwrap(),
                MicrophonePermissionStatus::new(MicrophonePermissionState::Missing, false)
            );
            assert_eq!(
                map_authorization_status(AVAuthorizationStatus::Restricted).unwrap(),
                MicrophonePermissionStatus::new(MicrophonePermissionState::Restricted, false)
            );
            assert_eq!(
                map_authorization_status(AVAuthorizationStatus::Authorized).unwrap(),
                MicrophonePermissionStatus::new(MicrophonePermissionState::Granted, false)
            );
        }

        #[test]
        fn rejects_unknown_authorization_states_instead_of_claiming_access() {
            let error = map_authorization_status(AVAuthorizationStatus(99)).unwrap_err();
            assert!(error.contains("unknown microphone authorization status"));
        }

        #[test]
        fn privacy_settings_link_targets_the_microphone_pane() {
            assert!(MICROPHONE_PRIVACY_SETTINGS_URL.starts_with("x-apple.systempreferences:"));
            assert!(MICROPHONE_PRIVACY_SETTINGS_URL.ends_with("Privacy_Microphone"));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::MicrophonePermissionStatus;

    pub(super) fn status() -> Result<MicrophonePermissionStatus, String> {
        Ok(MicrophonePermissionStatus::unsupported())
    }

    pub(super) fn request_access<F>(completion: F) -> Result<(), String>
    where
        F: Fn(Result<MicrophonePermissionStatus, String>) + Send + Sync + 'static,
    {
        completion(Ok(MicrophonePermissionStatus::unsupported()));
        Ok(())
    }

    pub(super) fn open_microphone_privacy_settings() -> Result<(), String> {
        Err("Microphone privacy settings are only available on macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_payload_uses_the_frontend_contract() {
        let value = serde_json::to_value(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Missing,
            true,
        ))
        .unwrap();

        assert_eq!(value["state"], "missing");
        assert_eq!(value["canRequest"], true);
        assert!(value.get("can_request").is_none());
    }

    #[test]
    fn capture_preflight_allows_only_granted_or_platform_managed_access() {
        assert!(validate_capture_status(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Granted,
            false,
        ))
        .is_ok());
        assert!(validate_capture_status(MicrophonePermissionStatus::unsupported()).is_ok());

        let not_requested = validate_capture_status(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Missing,
            true,
        ))
        .unwrap_err();
        assert!(not_requested.contains("Allow microphone"));

        let denied = validate_capture_status(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Missing,
            false,
        ))
        .unwrap_err();
        assert!(denied.contains("Privacy & Security → Microphone"));

        let restricted = validate_capture_status(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Restricted,
            false,
        ))
        .unwrap_err();
        assert!(restricted.contains("restricted"));
    }

    #[test]
    fn only_an_undetermined_permission_can_present_the_system_prompt() {
        assert!(should_request_access(MicrophonePermissionStatus::new(
            MicrophonePermissionState::Missing,
            true,
        )));
        for permission in [
            MicrophonePermissionStatus::new(MicrophonePermissionState::Missing, false),
            MicrophonePermissionStatus::new(MicrophonePermissionState::Restricted, false),
            MicrophonePermissionStatus::new(MicrophonePermissionState::Granted, false),
            MicrophonePermissionStatus::unsupported(),
        ] {
            assert!(!should_request_access(permission));
        }
    }
}
