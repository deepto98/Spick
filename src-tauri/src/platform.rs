//! Narrow platform seams for native text insertion.
//!
//! A focused field is captured before recording and represented everywhere
//! outside the operating-system adapter by a one-use opaque token. Commit never
//! falls back after a native write was attempted: an ambiguous result must not
//! risk typing the transcript twice.

use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(any(
    test,
    all(target_os = "macos", feature = "macos-input-method-prototype")
))]
mod input_method_protocol;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(all(
    target_os = "macos",
    feature = "macos-input-method-compatibility-harness"
))]
pub(crate) const INPUT_METHOD_PROTOCOL_VERSION: u8 = input_method_protocol::PROTOCOL_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopPlatform {
    MacOs,
    Windows,
    Linux,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextInsertionStrategy {
    InputMethodKit,
    Accessibility,
    KeyboardEvents,
    UiAutomation,
    AtSpi,
    ClipboardPaste,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub platform: DesktopPlatform,
    pub preferred_text_insertion: TextInsertionStrategy,
    /// Present only when a fallback is actually wired into this build.
    pub fallback_text_insertion: Option<TextInsertionStrategy>,
    pub text_insertion_available: bool,
    pub supports_global_shortcut: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessibilityPermissionState {
    Granted,
    Missing,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityPermissionStatus {
    pub state: AccessibilityPermissionState,
    pub can_request: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextTargetToken(u64);

impl TextTargetToken {
    #[cfg(any(not(target_os = "macos"), test))]
    const fn new(value: u64) -> Self {
        Self(value)
    }

    #[cfg(target_os = "macos")]
    pub(super) const fn from_platform(value: u64) -> Self {
        Self(value)
    }

    #[cfg(target_os = "macos")]
    pub(super) const fn platform_value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedTextTarget {
    pub token: TextTargetToken,
    /// Application name only. Field titles, values, selections, and native
    /// identifiers never cross the platform boundary.
    pub target_app: Option<String>,
    /// Used only inside the fixed-fixture harness to resolve public version
    /// metadata for the exact process captured by Accessibility. It is never
    /// serialized or included in evidence.
    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    pub(crate) compatibility_target_pid: libc::pid_t,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTargetErrorKind {
    AccessibilityMissing,
    NoFocusedTarget,
    OwnApplication,
    NotEditable,
    SecureField,
    ExpectedApplicationMismatch,
    ExpectedSelectionMismatch,
    Unsupported,
    FocusChanged,
    SelectionChanged,
    ContentChanged,
    TargetGone,
    TimedOut,
    Indeterminate,
    Platform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextTargetError {
    pub kind: TextTargetErrorKind,
    message: String,
    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    pub(crate) compatibility_target_pid: Option<libc::pid_t>,
}

impl TextTargetError {
    pub fn new(kind: TextTargetErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            #[cfg(all(
                target_os = "macos",
                feature = "macos-input-method-compatibility-harness"
            ))]
            compatibility_target_pid: None,
        }
    }

    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    pub(super) fn with_compatibility_target_pid(mut self, pid: libc::pid_t) -> Self {
        self.compatibility_target_pid = Some(pid);
        self
    }
}

impl fmt::Display for TextTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TextTargetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInsertionReceipt {
    pub target_app: Option<String>,
    /// The text was already inserted when this is false. A caret adjustment
    /// failure is diagnostic only and must never trigger a retry.
    pub caret_repositioned: bool,
    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    pub(crate) compatibility_peer_cd_hash: Option<String>,
}

/// Cloneable proxy for the operating-system owner thread.
///
/// Native accessibility references are deliberately absent from this type, so
/// Tauri commands and transcription workers cannot move them across threads.
#[derive(Default)]
pub struct TextTargetController {
    #[cfg(target_os = "macos")]
    inner: macos::MacTextTargetController,
}

impl TextTargetController {
    pub fn permission_status(&self) -> AccessibilityPermissionStatus {
        #[cfg(target_os = "macos")]
        {
            self.inner.permission_status()
        }

        #[cfg(not(target_os = "macos"))]
        {
            AccessibilityPermissionStatus {
                state: AccessibilityPermissionState::Unsupported,
                can_request: false,
            }
        }
    }

    pub fn request_permission(&self) -> Result<AccessibilityPermissionStatus, TextTargetError> {
        #[cfg(target_os = "macos")]
        {
            self.inner.request_permission()
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(self.permission_status())
        }
    }

    pub fn capture(&self) -> Result<CapturedTextTarget, TextTargetError> {
        #[cfg(target_os = "macos")]
        {
            self.inner.capture()
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "direct text insertion is not connected on this platform yet",
            ))
        }
    }

    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    pub(crate) fn capture_for_compatibility(
        &self,
        expected_bundle_identifier: &str,
        selection: CompatibilitySelection,
    ) -> Result<CapturedTextTarget, TextTargetError> {
        self.inner
            .capture_for_compatibility(expected_bundle_identifier, selection)
    }

    pub fn commit(
        &self,
        token: TextTargetToken,
        text: &str,
    ) -> Result<TextInsertionReceipt, TextTargetError> {
        #[cfg(target_os = "macos")]
        {
            self.inner.commit(token, text)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (token, text);
            Err(TextTargetError::new(
                TextTargetErrorKind::Unsupported,
                "direct text insertion is not connected on this platform yet",
            ))
        }
    }

    pub fn discard(&self, token: TextTargetToken) {
        #[cfg(target_os = "macos")]
        self.inner.discard(token);

        #[cfg(not(target_os = "macos"))]
        let _ = token;
    }
}

#[cfg(all(
    target_os = "macos",
    feature = "macos-input-method-compatibility-harness"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompatibilitySelection {
    Any,
    Caret,
    Range,
}

pub fn current_platform_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "macos")]
    {
        let development_accessibility_insertion = cfg!(debug_assertions);
        PlatformCapabilities {
            platform: DesktopPlatform::MacOs,
            preferred_text_insertion: if development_accessibility_insertion {
                TextInsertionStrategy::Accessibility
            } else {
                TextInsertionStrategy::InputMethodKit
            },
            fallback_text_insertion: development_accessibility_insertion
                .then_some(TextInsertionStrategy::ClipboardPaste),
            text_insertion_available: development_accessibility_insertion,
            supports_global_shortcut: true,
        }
    }

    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            platform: DesktopPlatform::Windows,
            preferred_text_insertion: TextInsertionStrategy::UiAutomation,
            fallback_text_insertion: None,
            text_insertion_available: false,
            supports_global_shortcut: true,
        }
    }

    #[cfg(target_os = "linux")]
    {
        // The current global-hotkey backend is X11-only. XWayland commonly
        // exposes DISPLAY but cannot provide a reliable system-wide shortcut
        // across native Wayland applications, so stay conservative there.
        let session_type = std::env::var("XDG_SESSION_TYPE").ok();
        let supports_global_shortcut = linux_x11_shortcuts_available(
            std::env::var_os("DISPLAY").is_some(),
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            session_type.as_deref(),
        );
        PlatformCapabilities {
            platform: DesktopPlatform::Linux,
            preferred_text_insertion: TextInsertionStrategy::AtSpi,
            fallback_text_insertion: None,
            text_insertion_available: false,
            supports_global_shortcut,
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        PlatformCapabilities {
            platform: DesktopPlatform::Unknown,
            preferred_text_insertion: TextInsertionStrategy::ClipboardPaste,
            fallback_text_insertion: None,
            text_insertion_available: false,
            supports_global_shortcut: false,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_x11_shortcuts_available(
    display_present: bool,
    wayland_display_present: bool,
    session_type: Option<&str>,
) -> bool {
    display_present
        && !wayland_display_present
        && !session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
}

#[cfg(test)]
mod tests {
    use super::{current_platform_capabilities, linux_x11_shortcuts_available, TextTargetToken};

    #[test]
    fn linux_shortcuts_require_x11_instead_of_xwayland_only() {
        assert!(linux_x11_shortcuts_available(true, false, Some("x11")));
        assert!(linux_x11_shortcuts_available(true, false, None));
        assert!(!linux_x11_shortcuts_available(false, false, Some("x11")));
        assert!(!linux_x11_shortcuts_available(true, true, None));
        assert!(!linux_x11_shortcuts_available(true, false, Some("wayland")));
        assert!(!linux_x11_shortcuts_available(true, false, Some("Wayland")));
    }

    #[test]
    fn target_tokens_are_opaque_but_stable_inside_the_native_core() {
        let token = TextTargetToken::new(42);
        assert_eq!(token, token);
        assert_ne!(token, TextTargetToken::new(43));
    }

    #[test]
    fn capabilities_advertise_only_wired_fallbacks() {
        let capabilities = current_platform_capabilities();
        #[cfg(target_os = "macos")]
        assert!(capabilities.text_insertion_available);
        #[cfg(not(target_os = "macos"))]
        assert!(!capabilities.text_insertion_available);
        #[cfg(target_os = "macos")]
        assert_eq!(
            capabilities.fallback_text_insertion,
            Some(super::TextInsertionStrategy::ClipboardPaste)
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(capabilities.fallback_text_insertion, None);
    }
}
