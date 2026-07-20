use std::cmp::Ordering;
#[cfg(target_os = "macos")]
use std::{
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering as AtomicOrdering},
        mpsc,
    },
    time::Duration,
};

#[cfg(target_os = "macos")]
use tauri::WebviewWindow;
use tauri::{
    AppHandle, LogicalSize, Manager, Monitor, PhysicalPosition, Runtime, WebviewUrl,
    WebviewWindowBuilder,
};
#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelHandle, PanelLevel, StyleMask,
    WebviewWindowExt,
};

use crate::domain::{HudCoordinates, HudPosition, HudPresentation, HudSettings};

pub const HUD_WINDOW_LABEL: &str = "hud";
const EXPANDED_HUD_WIDTH: f64 = 360.0;
const EXPANDED_HUD_HEIGHT: f64 = 104.0;
const COMPACT_HUD_WIDTH: f64 = 56.0;
const COMPACT_HUD_HEIGHT: f64 = 116.0;
const HUD_MARGIN: f64 = 24.0;
#[cfg(target_os = "macos")]
const PANEL_MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(target_os = "macos")]
static NATIVE_PANEL_READY: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static NATIVE_PANEL_TARGET_SAFE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static HUD_TARGET_PROTECTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static NATIVE_PANEL_WINDOW_NUMBER: AtomicIsize = AtomicIsize::new(0);

// NSWindowStyleMaskNonactivatingPanel only works for NSPanel subclasses. The
// class is installed once while the Tauri window is hidden and is never changed
// back; closing or re-converting a swizzled panel is intentionally unsupported.
#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(SpickHudPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorGeometry {
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
}

impl From<&Monitor> for MonitorGeometry {
    fn from(monitor: &Monitor) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        let work_area = monitor.work_area();
        Self {
            monitor_x: position.x,
            monitor_y: position.y,
            monitor_width: size.width,
            monitor_height: size.height,
            work_x: work_area.position.x,
            work_y: work_area.position.y,
            work_width: work_area.size.width,
            work_height: work_area.size.height,
            scale_factor: monitor.scale_factor(),
        }
    }
}

#[cfg(target_os = "macos")]
fn install_native_panel<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    if !is_macos_main_thread() {
        return Err("dictation HUD panel conversion must run on the macOS main thread".into());
    }
    let panel = window
        .to_panel::<SpickHudPanel<R>>()
        .map_err(|error| format!("could not convert the dictation HUD to NSPanel: {error}"))?;

    // Borderless must be applied before NonactivatingPanel because the
    // plugin's borderless builder resets the mask to zero.
    panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());
    panel.set_floating_panel(true);
    panel.set_level(PanelLevel::Floating.value());
    panel.set_hides_on_deactivate(false);
    panel.set_released_when_closed(false);
    panel.set_has_shadow(false);
    panel.set_ignores_mouse_events(false);
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .ignores_cycle()
            .into(),
    );

    let target_safe = !panel.can_become_key_window() && !panel.can_become_main_window();
    if !target_safe {
        eprintln!("native dictation panel unexpectedly accepts key or main window status");
    }

    remember_native_panel_window_number(&panel);
    HUD_TARGET_PROTECTED.store(false, AtomicOrdering::Release);
    NATIVE_PANEL_TARGET_SAFE.store(target_safe, AtomicOrdering::Release);
    NATIVE_PANEL_READY.store(true, AtomicOrdering::Release);
    Ok(())
}

#[cfg(target_os = "macos")]
fn remember_native_panel_window_number<R: Runtime>(panel: &PanelHandle<R>) {
    // SAFETY: callers run on AppKit's main thread and `as_panel` remains valid
    // for the lifetime of the Tauri-owned HUD. Hidden windows can report zero;
    // `show` refreshes the number after ordering the panel on screen.
    let window_number: isize =
        unsafe { tauri_nspanel::objc2::msg_send![panel.as_panel(), windowNumber] };
    NATIVE_PANEL_WINDOW_NUMBER.store(window_number.max(0), AtomicOrdering::Release);
}

/// Reports whether a CoreGraphics pointer event targets the live native HUD.
///
/// The Option listener uses the NSWindow number rather than broad process or
/// screen bounds, so clicks in Spick's dashboard or a nearby app do not receive
/// the HUD drag exemption.
#[cfg(target_os = "macos")]
pub(crate) fn owns_native_window_number(window_number: i64) -> bool {
    let hud_window_number = NATIVE_PANEL_WINDOW_NUMBER.load(AtomicOrdering::Acquire);
    hud_window_number > 0 && window_number == hud_window_number as i64
}

#[cfg(target_os = "macos")]
fn is_macos_main_thread() -> bool {
    // SAFETY: pthread_main_np has no preconditions and only inspects the
    // calling thread.
    unsafe { libc::pthread_main_np() != 0 }
}

#[cfg(target_os = "macos")]
fn with_native_panel_on_main_thread<R, F>(
    app: &AppHandle<R>,
    operation: &'static str,
    action: F,
) -> Result<bool, String>
where
    R: Runtime,
    F: FnOnce(PanelHandle<R>) -> Result<(), String> + Send + 'static,
{
    if !NATIVE_PANEL_READY.load(AtomicOrdering::Acquire) {
        return Ok(false);
    }

    if is_macos_main_thread() {
        let panel = app
            .get_webview_panel(HUD_WINDOW_LABEL)
            .map_err(|_| format!("native dictation panel is unavailable during {operation}"))?;
        action(panel)?;
        return Ok(true);
    }

    let app = app.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    app.clone()
        .run_on_main_thread(move || {
            let result = app
                .get_webview_panel(HUD_WINDOW_LABEL)
                .map_err(|_| format!("native dictation panel is unavailable during {operation}"))
                .and_then(action);
            let _ = sender.send(result);
        })
        .map_err(|error| format!("could not schedule dictation panel {operation}: {error}"))?;

    receiver
        .recv_timeout(PANEL_MAIN_THREAD_TIMEOUT)
        .map_err(|_| format!("timed out waiting for dictation panel {operation}"))??;
    Ok(true)
}

pub fn create<R: Runtime>(app: &AppHandle<R>, settings: &HudSettings) -> Result<(), String> {
    if app.get_webview_window(HUD_WINDOW_LABEL).is_some() {
        return apply(app, settings);
    }

    let (width, height) = logical_dimensions(settings.presentation);
    let window = WebviewWindowBuilder::new(
        app,
        HUD_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=hud".into()),
    )
    .title("Spick Dictation")
    .inner_size(width, height)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .prevent_overflow_with_margin(LogicalSize::new(HUD_MARGIN, HUD_MARGIN))
    .build()
    .map_err(|error| format!("could not create dictation HUD: {error}"))?;

    if let Some(position) = position_for_settings(app, settings)? {
        window
            .set_position(position)
            .map_err(|error| format!("could not position dictation HUD: {error}"))?;
    }

    #[cfg(target_os = "macos")]
    if let Err(error) = install_native_panel(&window) {
        // A normal Tauri window is not guaranteed to remain nonactivating when
        // clicked. Fail closed until a caller explicitly shows an idle HUD.
        NATIVE_PANEL_READY.store(false, AtomicOrdering::Release);
        NATIVE_PANEL_WINDOW_NUMBER.store(0, AtomicOrdering::Release);
        HUD_TARGET_PROTECTED.store(true, AtomicOrdering::Release);
        window
            .set_ignore_cursor_events(true)
            .map_err(|fallback_error| {
                format!(
                    "{error}; could not make the fallback HUD pointer-through: {fallback_error}"
                )
            })?;
        eprintln!("{error}; using the pointer-through HUD fallback");
    }

    Ok(())
}

/// Applies both the persisted presentation and placement to the native HUD.
///
/// The native window is resized along with its React surface so a compact HUD
/// does not leave a transparent rectangle intercepting the target app's input.
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &HudSettings) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    let (width, height) = logical_dimensions(settings.presentation);

    #[cfg(target_os = "macos")]
    let resized_as_panel = with_native_panel_on_main_thread(app, "resize", move |panel| {
        panel.set_content_size(width, height);
        Ok(())
    })?;
    #[cfg(not(target_os = "macos"))]
    let resized_as_panel = false;

    if !resized_as_panel {
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|error| format!("could not resize dictation HUD: {error}"))?;
    }

    reposition(app, settings)
}

pub fn reposition<R: Runtime>(app: &AppHandle<R>, settings: &HudSettings) -> Result<(), String> {
    let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) else {
        return Err("dictation HUD is not available".into());
    };
    let Some(position) = position_for_settings(app, settings)? else {
        return Ok(());
    };

    window
        .set_position(position)
        .map_err(|error| format!("could not position dictation HUD: {error}"))
}

pub fn show<R: Runtime>(
    app: &AppHandle<R>,
    settings: &HudSettings,
    target_is_live: bool,
) -> Result<(), String> {
    // Monitor enumeration can transiently fail while a display is attached or
    // removed. Keep the last valid geometry and still provide capture feedback.
    if let Err(error) = apply(app, settings) {
        eprintln!("showing the dictation HUD with its last geometry: {error}");
    }
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let shown_as_panel = with_native_panel_on_main_thread(app, "show", move |panel| {
            let protect = target_is_live && !NATIVE_PANEL_TARGET_SAFE.load(AtomicOrdering::Acquire);
            panel.set_ignores_mouse_events(protect);
            // An existing hidden NSPanel normally retains its window number.
            // Publish it before ordering the panel front to close the tiny gap
            // in which a very fast move-grip press could otherwise look external.
            remember_native_panel_window_number(&panel);
            panel.show();
            remember_native_panel_window_number(&panel);
            Ok(())
        })?;
        if shown_as_panel {
            HUD_TARGET_PROTECTED.store(
                target_is_live && !NATIVE_PANEL_TARGET_SAFE.load(AtomicOrdering::Acquire),
                AtomicOrdering::Release,
            );
            return Ok(());
        }

        // A normal NSWindow may activate Spick when clicked even when it cannot
        // become key. Protect a captured target by making the fallback HUD
        // pointer-through for the entire target lifetime.
        window
            .set_ignore_cursor_events(target_is_live)
            .map_err(|error| format!("could not protect the fallback HUD target: {error}"))?;
        HUD_TARGET_PROTECTED.store(target_is_live, AtomicOrdering::Release);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = target_is_live;

    window
        .show()
        .map_err(|error| format!("could not show dictation HUD: {error}"))
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let hidden_as_panel = with_native_panel_on_main_thread(app, "hide", |panel| {
            panel.hide();
            NATIVE_PANEL_WINDOW_NUMBER.store(0, AtomicOrdering::Release);
            Ok(())
        })?;
        if hidden_as_panel {
            HUD_TARGET_PROTECTED.store(false, AtomicOrdering::Release);
            return Ok(());
        }
    }

    window
        .hide()
        .map_err(|error| format!("could not hide dictation HUD: {error}"))?;

    #[cfg(target_os = "macos")]
    {
        // Restore idle interaction only after the fallback is no longer
        // visible, leaving no click window in which the captured app can lose
        // focus.
        window
            .set_ignore_cursor_events(false)
            .map_err(|error| format!("could not restore fallback HUD interaction: {error}"))?;
        HUD_TARGET_PROTECTED.store(false, AtomicOrdering::Release);
    }

    Ok(())
}

/// Updates fallback click-through behavior when a captured insertion target is
/// acquired or released. A real nonactivating panel remains interactive.
pub fn protect_target<R: Runtime>(app: &AppHandle<R>, protect: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if NATIVE_PANEL_READY.load(AtomicOrdering::Acquire) {
            if NATIVE_PANEL_TARGET_SAFE.load(AtomicOrdering::Acquire) {
                return Ok(());
            }
            with_native_panel_on_main_thread(app, "target protection", move |panel| {
                panel.set_ignores_mouse_events(protect);
                Ok(())
            })?;
            HUD_TARGET_PROTECTED.store(protect, AtomicOrdering::Release);
            return Ok(());
        }
        let window = app
            .get_webview_window(HUD_WINDOW_LABEL)
            .ok_or_else(|| "dictation HUD is not available".to_string())?;
        window
            .set_ignore_cursor_events(protect)
            .map_err(|error| format!("could not update fallback HUD target protection: {error}"))?;
        HUD_TARGET_PROTECTED.store(protect, AtomicOrdering::Release);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (app, protect);

    Ok(())
}

/// Starts an operating-system window drag without making the HUD focusable.
///
/// The return value reports whether the native operation completed before this
/// function returned. macOS panel dragging is synchronous, which lets callers
/// persist the final position before another presentation change can restore a
/// stale coordinate. Tauri's portable fallback may continue asynchronously.
pub fn start_drag<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;

    #[cfg(target_os = "macos")]
    {
        if HUD_TARGET_PROTECTED.load(AtomicOrdering::Acquire) {
            return Err("the HUD cannot move while it is protecting a text target".into());
        }
        let dragged_as_panel = with_native_panel_on_main_thread(app, "drag", |panel| {
            // SAFETY: this closure is guaranteed to run on AppKit's main
            // thread. The event is borrowed for the duration of the synchronous
            // performWindowDragWithEvent: call.
            unsafe {
                let application: *mut tauri_nspanel::AnyObject = tauri_nspanel::objc2::msg_send![
                    tauri_nspanel::objc2::class!(NSApplication),
                    sharedApplication
                ];
                let event: *mut tauri_nspanel::AnyObject =
                    tauri_nspanel::objc2::msg_send![application, currentEvent];
                if event.is_null() {
                    return Err(
                        "macOS did not provide the mouse event needed to move the HUD".into(),
                    );
                }
                let _: () = tauri_nspanel::objc2::msg_send![
                    panel.as_panel(),
                    performWindowDragWithEvent: event
                ];
            }
            Ok(())
        })?;
        if dragged_as_panel {
            return Ok(true);
        }
    }

    window
        .start_dragging()
        .map_err(|error| format!("could not move dictation HUD: {error}"))?;
    Ok(false)
}

/// Returns the HUD's current physical top-left coordinate for persistence.
pub fn current_position<R: Runtime>(app: &AppHandle<R>) -> Result<HudCoordinates, String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    window
        .outer_position()
        .map(|position| HudCoordinates {
            x: position.x,
            y: position.y,
        })
        .map_err(|error| format!("could not read dictation HUD position: {error}"))
}

fn position_for_settings<R: Runtime>(
    app: &AppHandle<R>,
    settings: &HudSettings,
) -> Result<Option<PhysicalPosition<i32>>, String> {
    let monitors = app
        .available_monitors()
        .map_err(|error| format!("could not inspect available monitors: {error}"))?;
    let geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();

    if let Some(custom_position) = settings.custom_position {
        return Ok(
            resolve_custom_position(custom_position, settings.presentation, &geometries)
                .map(|position| PhysicalPosition::new(position.x, position.y)),
        );
    }

    let cursor = app.cursor_position().ok();
    let cursor_monitor = cursor.and_then(|cursor| {
        geometries.iter().copied().find(|monitor| {
            contains_physical_point(
                cursor.x,
                cursor.y,
                monitor.monitor_x,
                monitor.monitor_y,
                monitor.monitor_width,
                monitor.monitor_height,
            )
        })
    });
    let monitor = match cursor_monitor {
        Some(monitor) => Some(monitor),
        None => app
            .primary_monitor()
            .map_err(|error| format!("could not inspect the primary monitor: {error}"))?
            .as_ref()
            .map(MonitorGeometry::from)
            .or_else(|| geometries.first().copied()),
    };
    let Some(monitor) = monitor else {
        return Ok(None);
    };

    let (x, y) = preset_coordinates(monitor, settings.presentation, settings.position);
    Ok(Some(PhysicalPosition::new(x, y)))
}

fn logical_dimensions(presentation: HudPresentation) -> (f64, f64) {
    match presentation {
        HudPresentation::Expanded => (EXPANDED_HUD_WIDTH, EXPANDED_HUD_HEIGHT),
        HudPresentation::Compact => (COMPACT_HUD_WIDTH, COMPACT_HUD_HEIGHT),
    }
}

fn physical_dimensions(presentation: HudPresentation, scale_factor: f64) -> (u32, u32) {
    let (width, height) = logical_dimensions(presentation);
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    (
        (width * scale_factor).round().max(1.0) as u32,
        (height * scale_factor).round().max(1.0) as u32,
    )
}

fn resolve_custom_position(
    desired: HudCoordinates,
    presentation: HudPresentation,
    monitors: &[MonitorGeometry],
) -> Option<HudCoordinates> {
    let monitor = closest_monitor(desired, monitors)?;
    let (window_width, window_height) = physical_dimensions(presentation, monitor.scale_factor);
    Some(clamp_to_work_area(
        desired,
        monitor,
        window_width,
        window_height,
    ))
}

fn closest_monitor(
    desired: HudCoordinates,
    monitors: &[MonitorGeometry],
) -> Option<MonitorGeometry> {
    if let Some(containing) = monitors.iter().copied().find(|monitor| {
        contains_physical_point(
            f64::from(desired.x),
            f64::from(desired.y),
            monitor.monitor_x,
            monitor.monitor_y,
            monitor.monitor_width,
            monitor.monitor_height,
        )
    }) {
        return Some(containing);
    }

    monitors.iter().copied().min_by(|left, right| {
        distance_to_monitor(desired, *left)
            .partial_cmp(&distance_to_monitor(desired, *right))
            .unwrap_or(Ordering::Equal)
    })
}

fn distance_to_monitor(point: HudCoordinates, monitor: MonitorGeometry) -> f64 {
    let left = f64::from(monitor.monitor_x);
    let top = f64::from(monitor.monitor_y);
    let right = left + f64::from(monitor.monitor_width);
    let bottom = top + f64::from(monitor.monitor_height);
    let point_x = f64::from(point.x);
    let point_y = f64::from(point.y);
    let dx = if point_x < left {
        left - point_x
    } else if point_x >= right {
        point_x - right
    } else {
        0.0
    };
    let dy = if point_y < top {
        top - point_y
    } else if point_y >= bottom {
        point_y - bottom
    } else {
        0.0
    };
    dx.mul_add(dx, dy * dy)
}

fn clamp_to_work_area(
    desired: HudCoordinates,
    monitor: MonitorGeometry,
    window_width: u32,
    window_height: u32,
) -> HudCoordinates {
    let work_left = i64::from(monitor.work_x);
    let work_top = i64::from(monitor.work_y);
    let work_right = work_left + i64::from(monitor.work_width);
    let work_bottom = work_top + i64::from(monitor.work_height);
    let maximum_x = (work_right - i64::from(window_width)).max(work_left);
    let maximum_y = (work_bottom - i64::from(window_height)).max(work_top);

    HudCoordinates {
        x: clamp_i64_to_i32(i64::from(desired.x).clamp(work_left, maximum_x)),
        y: clamp_i64_to_i32(i64::from(desired.y).clamp(work_top, maximum_y)),
    }
}

fn clamp_i64_to_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn contains_physical_point(
    point_x: f64,
    point_y: f64,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
) -> bool {
    let left = f64::from(monitor_x);
    let top = f64::from(monitor_y);
    point_x >= left
        && point_x < left + f64::from(monitor_width)
        && point_y >= top
        && point_y < top + f64::from(monitor_height)
}

fn preset_coordinates(
    monitor: MonitorGeometry,
    presentation: HudPresentation,
    preferred_position: HudPosition,
) -> (i32, i32) {
    let work_x = f64::from(monitor.work_x);
    let work_y = f64::from(monitor.work_y);
    let work_width = f64::from(monitor.work_width);
    let work_height = f64::from(monitor.work_height);
    let (hud_width, hud_height) = physical_dimensions(presentation, monitor.scale_factor);
    let hud_width = f64::from(hud_width);
    let hud_height = f64::from(hud_height);
    let margin = HUD_MARGIN * monitor.scale_factor;
    let x = match preferred_position {
        HudPosition::BottomLeft => work_x + margin,
        HudPosition::BottomCenter => work_x + (work_width - hud_width) / 2.0,
        HudPosition::BottomRight => work_x + work_width - hud_width - margin,
    };
    let y = work_y + work_height - hud_height - margin;
    (x.round() as i32, y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(
        monitor: (i32, i32, u32, u32),
        work: (i32, i32, u32, u32),
        scale_factor: f64,
    ) -> MonitorGeometry {
        MonitorGeometry {
            monitor_x: monitor.0,
            monitor_y: monitor.1,
            monitor_width: monitor.2,
            monitor_height: monitor.3,
            work_x: work.0,
            work_y: work.1,
            work_width: work.2,
            work_height: work.3,
            scale_factor,
        }
    }

    #[test]
    fn computes_bottom_center_inside_the_work_area() {
        assert_eq!(
            preset_coordinates(
                monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0),
                HudPresentation::Expanded,
                HudPosition::BottomCenter,
            ),
            (540, 772)
        );
    }

    #[test]
    fn preserves_negative_monitor_origins() {
        assert_eq!(
            preset_coordinates(
                monitor((-1920, 0, 1920, 1080), (-1920, 0, 1920, 1080), 1.0,),
                HudPresentation::Expanded,
                HudPosition::BottomRight,
            ),
            (-384, 952)
        );
    }

    #[test]
    fn uses_the_target_monitor_scale_for_mixed_dpi_layouts() {
        assert_eq!(
            preset_coordinates(
                monitor((2880, 0, 3840, 2160), (2880, 0, 3840, 2160), 2.0,),
                HudPresentation::Expanded,
                HudPosition::BottomRight,
            ),
            (5952, 1904)
        );
    }

    #[test]
    fn compact_preset_uses_the_small_native_window_size() {
        assert_eq!(
            preset_coordinates(
                monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0),
                HudPresentation::Compact,
                HudPosition::BottomRight,
            ),
            (1360, 760)
        );
    }

    #[test]
    fn identifies_the_monitor_under_a_cursor_with_negative_origins() {
        assert!(contains_physical_point(
            -1200.0, 400.0, -1920, 0, 1920, 1080
        ));
        assert!(!contains_physical_point(20.0, 400.0, -1920, 0, 1920, 1080));
    }

    #[test]
    fn preserves_a_valid_custom_position_exactly() {
        let screens = [monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0)];
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: 602, y: 311 },
                HudPresentation::Expanded,
                &screens,
            ),
            Some(HudCoordinates { x: 602, y: 311 })
        );
    }

    #[test]
    fn clamps_a_custom_position_inside_its_monitor_work_area() {
        let screens = [monitor(
            (-1920, 0, 1920, 1080),
            (-1920, 24, 1920, 1056),
            1.0,
        )];
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: -2050, y: -40 },
                HudPresentation::Expanded,
                &screens,
            ),
            Some(HudCoordinates { x: -1920, y: 24 })
        );
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: -100, y: 1040 },
                HudPresentation::Expanded,
                &screens,
            ),
            Some(HudCoordinates { x: -360, y: 976 })
        );
    }

    #[test]
    fn clamps_using_compact_dimensions_on_retina_displays() {
        let screens = [monitor((0, 0, 3024, 1964), (0, 48, 3024, 1916), 2.0)];
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: 3000, y: 1900 },
                HudPresentation::Compact,
                &screens,
            ),
            Some(HudCoordinates { x: 2912, y: 1732 })
        );
    }

    #[test]
    fn removed_monitor_coordinates_fall_back_to_the_nearest_screen() {
        let screens = [
            monitor((-1920, 0, 1920, 1080), (-1920, 24, 1920, 1056), 1.0),
            monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0),
        ];
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: 5000, y: 300 },
                HudPresentation::Expanded,
                &screens,
            ),
            Some(HudCoordinates { x: 1080, y: 300 })
        );
    }

    #[test]
    fn adjacent_monitor_boundary_belongs_to_the_monitor_on_its_right() {
        let screens = [
            monitor((-1920, 0, 1920, 1080), (-1920, 24, 1920, 1056), 1.0),
            monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0),
        ];
        assert_eq!(
            resolve_custom_position(
                HudCoordinates { x: 0, y: 200 },
                HudPresentation::Compact,
                &screens,
            ),
            Some(HudCoordinates { x: 0, y: 200 })
        );
    }

    #[test]
    fn invalid_scale_factors_use_one_for_safe_geometry() {
        assert_eq!(
            physical_dimensions(HudPresentation::Compact, f64::NAN),
            (56, 116)
        );
        assert_eq!(
            physical_dimensions(HudPresentation::Expanded, 0.0),
            (360, 104)
        );
    }
}
