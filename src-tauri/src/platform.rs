//! Narrow platform seams for the native text-insertion work that follows.
//!
//! The dictation engine should only depend on these contracts. Each desktop OS
//! can then use its strongest accessibility API and a clipboard fallback without
//! leaking conditional compilation through the rest of the application.

use serde::{Deserialize, Serialize};

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
    Accessibility,
    UiAutomation,
    AtSpi,
    ClipboardPaste,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub platform: DesktopPlatform,
    pub preferred_text_insertion: TextInsertionStrategy,
    pub fallback_text_insertion: TextInsertionStrategy,
    pub supports_global_shortcut: bool,
}

pub fn current_platform_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "macos")]
    {
        PlatformCapabilities {
            platform: DesktopPlatform::MacOs,
            preferred_text_insertion: TextInsertionStrategy::Accessibility,
            fallback_text_insertion: TextInsertionStrategy::ClipboardPaste,
            supports_global_shortcut: true,
        }
    }

    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            platform: DesktopPlatform::Windows,
            preferred_text_insertion: TextInsertionStrategy::UiAutomation,
            fallback_text_insertion: TextInsertionStrategy::ClipboardPaste,
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
            fallback_text_insertion: TextInsertionStrategy::ClipboardPaste,
            supports_global_shortcut,
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        PlatformCapabilities {
            platform: DesktopPlatform::Unknown,
            preferred_text_insertion: TextInsertionStrategy::ClipboardPaste,
            fallback_text_insertion: TextInsertionStrategy::ClipboardPaste,
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

/// Contract implemented by the macOS, Windows, and Linux native adapters.
pub trait TextInputAdapter: Send + Sync {
    fn can_insert_into_focused_field(&self) -> Result<bool, String>;
    fn insert_text(&self, text: &str) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::linux_x11_shortcuts_available;

    #[test]
    fn linux_shortcuts_require_x11_instead_of_xwayland_only() {
        assert!(linux_x11_shortcuts_available(true, false, Some("x11")));
        assert!(linux_x11_shortcuts_available(true, false, None));
        assert!(!linux_x11_shortcuts_available(false, false, Some("x11")));
        assert!(!linux_x11_shortcuts_available(true, true, None));
        assert!(!linux_x11_shortcuts_available(true, false, Some("wayland")));
        assert!(!linux_x11_shortcuts_available(true, false, Some("Wayland")));
    }
}
