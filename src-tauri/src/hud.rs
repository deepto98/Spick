use std::{cmp::Ordering, sync::Mutex};
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
// These frames closely hug the largest rendered content. Compact keeps only a
// three-pixel transparent buffer for antialiasing instead of a broad invisible
// hit target over the app beneath it.
const EXPANDED_HUD_WIDTH: f64 = 336.0;
const EXPANDED_HUD_HEIGHT: f64 = 96.0;
const COMPACT_HUD_WIDTH: f64 = 48.0;
const COMPACT_HUD_HEIGHT: f64 = 110.0;
const HUD_MARGIN: f64 = 24.0;
#[cfg(target_os = "macos")]
const PANEL_MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const NS_EVENT_TYPE_LEFT_MOUSE_DOWN: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingHudShow {
    request_id: u64,
    target_is_live: bool,
}

#[derive(Debug, Default)]
struct HudLifecycleState {
    renderer_ready: bool,
    show_requested: bool,
    target_is_live: bool,
    request_id: u64,
    shown_target_is_live: Option<bool>,
    applied_settings: Option<HudSettings>,
    settings_apply_id: u64,
}

impl HudLifecycleState {
    fn reset_for_create(&mut self) {
        // Keep generations monotonic so a completion from a superseded native
        // window cannot acknowledge state for its replacement.
        self.request_id = self.request_id.wrapping_add(1);
        self.settings_apply_id = self.settings_apply_id.wrapping_add(1);
        self.renderer_ready = false;
        self.show_requested = false;
        self.target_is_live = false;
        self.shown_target_is_live = None;
        self.applied_settings = None;
    }

    fn try_request_steady_state_transition(
        &mut self,
        settings: &HudSettings,
        target_is_live: bool,
        target_transition_is_safe: bool,
    ) -> bool {
        if !self.can_use_steady_state_fast_path(settings, target_is_live, target_transition_is_safe)
        {
            return false;
        }

        self.record_show_request(target_is_live);
        // A target-safe panel has no native state to change between idle and
        // live. The renderer receives its own session event, so the
        // already-visible native window is fully up to date here.
        self.shown_target_is_live = Some(target_is_live);
        true
    }

    fn request_show_after_apply(&mut self, target_is_live: bool) -> Option<PendingHudShow> {
        let pending = self.record_show_request(target_is_live);
        self.renderer_ready.then_some(pending)
    }

    fn record_show_request(&mut self, target_is_live: bool) -> PendingHudShow {
        self.request_id = self.request_id.wrapping_add(1);
        self.show_requested = true;
        self.target_is_live = target_is_live;
        PendingHudShow {
            request_id: self.request_id,
            target_is_live,
        }
    }

    fn can_use_steady_state_fast_path(
        &self,
        settings: &HudSettings,
        target_is_live: bool,
        target_transition_is_safe: bool,
    ) -> bool {
        self.renderer_ready
            && self.show_requested
            && target_transition_is_safe
            && self.applied_settings.as_ref() == Some(settings)
            && self
                .shown_target_is_live
                .is_some_and(|shown_target| shown_target != target_is_live)
    }

    fn request_hide(&mut self) {
        self.request_id = self.request_id.wrapping_add(1);
        self.show_requested = false;
        self.target_is_live = false;
        self.shown_target_is_live = None;
        self.invalidate_applied_settings();
    }

    fn mark_renderer_ready(&mut self) -> Option<PendingHudShow> {
        self.renderer_ready = true;
        self.show_requested.then_some(PendingHudShow {
            request_id: self.request_id,
            target_is_live: self.target_is_live,
        })
    }

    fn begin_show(&mut self, pending: PendingHudShow) -> bool {
        let is_current = self.renderer_ready
            && self.show_requested
            && self.request_id == pending.request_id
            && self.target_is_live == pending.target_is_live;
        if is_current {
            // A failure from this point onward must force the next request
            // through the full native path.
            self.shown_target_is_live = None;
        }
        is_current
    }

    fn acknowledge_show(&mut self, pending: PendingHudShow) {
        if self.renderer_ready
            && self.show_requested
            && self.request_id == pending.request_id
            && self.target_is_live == pending.target_is_live
        {
            self.shown_target_is_live = Some(pending.target_is_live);
        }
    }

    fn begin_settings_apply(&mut self) -> u64 {
        self.settings_apply_id = self.settings_apply_id.wrapping_add(1);
        self.applied_settings = None;
        self.settings_apply_id
    }

    fn acknowledge_settings_apply(&mut self, apply_id: u64, settings: &HudSettings) {
        if self.settings_apply_id == apply_id {
            self.applied_settings = Some(settings.clone());
        }
    }

    fn invalidate_applied_settings(&mut self) {
        self.settings_apply_id = self.settings_apply_id.wrapping_add(1);
        self.applied_settings = None;
    }
}

static HUD_LIFECYCLE: Mutex<HudLifecycleState> = Mutex::new(HudLifecycleState {
    renderer_ready: false,
    show_requested: false,
    target_is_live: false,
    request_id: 0,
    shown_target_is_live: None,
    applied_settings: None,
    settings_apply_id: 0,
});

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

fn reset_renderer_gate() -> Result<(), String> {
    let mut lifecycle = HUD_LIFECYCLE
        .lock()
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())?;
    lifecycle.reset_for_create();
    Ok(())
}

fn try_steady_state_transition(
    settings: &HudSettings,
    target_is_live: bool,
) -> Result<bool, String> {
    HUD_LIFECYCLE
        .lock()
        .map(|mut lifecycle| {
            lifecycle.try_request_steady_state_transition(
                settings,
                target_is_live,
                target_transition_can_skip_native_operations(),
            )
        })
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())
}

fn request_show_after_apply(target_is_live: bool) -> Result<Option<PendingHudShow>, String> {
    HUD_LIFECYCLE
        .lock()
        .map(|mut lifecycle| lifecycle.request_show_after_apply(target_is_live))
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())
}

#[cfg(target_os = "macos")]
fn target_transition_can_skip_native_operations() -> bool {
    NATIVE_PANEL_READY.load(AtomicOrdering::Acquire)
        && NATIVE_PANEL_TARGET_SAFE.load(AtomicOrdering::Acquire)
}

#[cfg(not(target_os = "macos"))]
fn target_transition_can_skip_native_operations() -> bool {
    true
}

fn begin_settings_apply() -> Result<u64, String> {
    HUD_LIFECYCLE
        .lock()
        .map(|mut lifecycle| lifecycle.begin_settings_apply())
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())
}

fn acknowledge_settings_apply(apply_id: u64, settings: &HudSettings) -> Result<(), String> {
    let mut lifecycle = HUD_LIFECYCLE
        .lock()
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())?;
    lifecycle.acknowledge_settings_apply(apply_id, settings);
    Ok(())
}

fn begin_show(pending: PendingHudShow) -> Result<bool, String> {
    HUD_LIFECYCLE
        .lock()
        .map(|mut lifecycle| lifecycle.begin_show(pending))
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())
}

fn acknowledge_show(pending: PendingHudShow) -> Result<(), String> {
    let mut lifecycle = HUD_LIFECYCLE
        .lock()
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())?;
    lifecycle.acknowledge_show(pending);
    Ok(())
}

fn request_hide() -> Result<(), String> {
    let mut lifecycle = HUD_LIFECYCLE
        .lock()
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())?;
    lifecycle.request_hide();
    Ok(())
}

pub fn create<R: Runtime>(app: &AppHandle<R>, settings: &HudSettings) -> Result<(), String> {
    if app.get_webview_window(HUD_WINDOW_LABEL).is_some() {
        return apply(app, settings);
    }

    // The native window may finish building before its React surface has read
    // the persisted presentation. Keep it hidden until that surface explicitly
    // confirms hydration, otherwise a compact native frame briefly contains
    // the expanded widget (or remains clipped if IPC initialization fails).
    reset_renderer_gate()?;

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
        // clicked. Keep this fallback display-only for the process lifetime.
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
    // Invalidate before the first native mutation. A partial resize followed by
    // a placement failure must never leave the previous settings eligible for
    // the steady-state fast path.
    let apply_id = begin_settings_apply()?;
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

    if reposition_native(app, settings)? {
        acknowledge_settings_apply(apply_id, settings)?;
    }
    Ok(())
}

fn reposition_native<R: Runtime>(
    app: &AppHandle<R>,
    settings: &HudSettings,
) -> Result<bool, String> {
    let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) else {
        return Err("dictation HUD is not available".into());
    };
    let Some(position) = position_for_settings(app, settings)? else {
        return Ok(false);
    };

    window
        .set_position(position)
        .map_err(|error| format!("could not position dictation HUD: {error}"))?;
    Ok(true)
}

pub fn show<R: Runtime>(
    app: &AppHandle<R>,
    settings: &HudSettings,
    target_is_live: bool,
) -> Result<(), String> {
    if try_steady_state_transition(settings, target_is_live)? {
        return Ok(());
    }

    // Monitor enumeration can transiently fail while a display is attached or
    // removed. Keep the last valid geometry and still provide capture feedback.
    if let Err(error) = apply(app, settings) {
        eprintln!("showing the dictation HUD with its last geometry: {error}");
    }
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    let Some(pending) = request_show_after_apply(target_is_live)? else {
        return Ok(());
    };

    show_ready_window(app, window, pending)
}

fn show_ready_window<R: Runtime>(
    app: &AppHandle<R>,
    window: WebviewWindow<R>,
    pending: PendingHudShow,
) -> Result<(), String> {
    if !begin_show(pending)? {
        return Ok(());
    }
    let target_is_live = pending.target_is_live;

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
            acknowledge_show(pending)?;
            return Ok(());
        }

        // A normal NSWindow may activate Spick before a target can be captured,
        // including from the idle widget. A failed NSPanel conversion therefore
        // degrades to a display-only, shortcut-driven HUD for its entire life.
        window
            .set_ignore_cursor_events(true)
            .map_err(|error| format!("could not protect the fallback HUD target: {error}"))?;
        HUD_TARGET_PROTECTED.store(true, AtomicOrdering::Release);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = target_is_live;

    window
        .show()
        .map_err(|error| format!("could not show dictation HUD: {error}"))?;
    acknowledge_show(pending)
}

/// Releases the startup visibility gate after the HUD renderer has committed
/// its persisted presentation. Repeated calls are harmless and can retry a
/// transient native presentation failure.
pub fn mark_renderer_ready<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let pending_target = HUD_LIFECYCLE
        .lock()
        .map_err(|_| "dictation HUD lifecycle is unavailable".to_string())?
        .mark_renderer_ready();
    let Some(pending) = pending_target else {
        return Ok(());
    };
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    show_ready_window(app, window, pending)
}

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    app.get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?
        .is_visible()
        .map_err(|error| format!("could not read dictation HUD visibility: {error}"))
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    request_hide()?;
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
        // A fallback NSWindow is never interactive: even an idle click could
        // activate Spick before the intended external target is captured.
        window
            .set_ignore_cursor_events(true)
            .map_err(|error| format!("could not restore fallback HUD interaction: {error}"))?;
        HUD_TARGET_PROTECTED.store(true, AtomicOrdering::Release);
    }

    Ok(())
}

/// Keeps an unsafe fallback click-through when target ownership changes. A real
/// nonactivating panel remains interactive because it cannot steal activation.
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
            .set_ignore_cursor_events(true)
            .map_err(|error| format!("could not update fallback HUD target protection: {error}"))?;
        HUD_TARGET_PROTECTED.store(true, AtomicOrdering::Release);
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
                let event_type: usize = tauri_nspanel::objc2::msg_send![event, type];
                let event_window_number: isize =
                    tauri_nspanel::objc2::msg_send![event, windowNumber];
                let panel_window_number: isize =
                    tauri_nspanel::objc2::msg_send![panel.as_panel(), windowNumber];
                if !is_native_drag_event(event_type, event_window_number, panel_window_number) {
                    return Err(
                        "macOS did not report a left-button press on the HUD for this drag".into(),
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

#[cfg(target_os = "macos")]
fn is_native_drag_event(
    event_type: usize,
    event_window_number: isize,
    panel_window_number: isize,
) -> bool {
    event_type == NS_EVENT_TYPE_LEFT_MOUSE_DOWN
        && panel_window_number > 0
        && event_window_number == panel_window_number
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

    fn settle_visible_hud(
        lifecycle: &mut HudLifecycleState,
        settings: &HudSettings,
        target_is_live: bool,
    ) {
        let apply_id = lifecycle.begin_settings_apply();
        lifecycle.acknowledge_settings_apply(apply_id, settings);
        assert_eq!(lifecycle.request_show_after_apply(target_is_live), None);
        let pending = lifecycle
            .mark_renderer_ready()
            .expect("the queued show should remain current");
        assert!(lifecycle.begin_show(pending));
        lifecycle.acknowledge_show(pending);
    }

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
    fn renderer_gate_queues_the_latest_show_until_hydration() {
        let mut lifecycle = HudLifecycleState::default();

        assert_eq!(lifecycle.request_show_after_apply(true), None,);
        assert_eq!(lifecycle.request_show_after_apply(false), None,);
        assert_eq!(
            lifecycle.mark_renderer_ready(),
            Some(PendingHudShow {
                request_id: 2,
                target_is_live: false,
            })
        );
        assert_eq!(
            lifecycle.request_show_after_apply(true),
            Some(PendingHudShow {
                request_id: 3,
                target_is_live: true,
            })
        );
    }

    #[test]
    fn hiding_before_hydration_cancels_the_queued_show() {
        let mut lifecycle = HudLifecycleState::default();

        assert_eq!(lifecycle.request_show_after_apply(true), None);
        lifecycle.request_hide();

        assert_eq!(lifecycle.mark_renderer_ready(), None);
    }

    #[test]
    fn acknowledged_target_only_transition_uses_the_fast_path() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        assert!(lifecycle.try_request_steady_state_transition(&settings, true, true));
        assert_eq!(lifecycle.shown_target_is_live, Some(true));
        assert!(lifecycle.try_request_steady_state_transition(&settings, false, true));
    }

    #[test]
    fn fast_path_rejects_changed_settings_and_same_target_requests() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        assert!(!lifecycle.try_request_steady_state_transition(&settings, false, true));
        let mut changed = settings.clone();
        changed.presentation = HudPresentation::Expanded;
        assert!(!lifecycle.try_request_steady_state_transition(&changed, true, true));

        let mut moved = settings;
        moved.custom_position = Some(HudCoordinates { x: 120, y: 80 });
        assert!(!lifecycle.try_request_steady_state_transition(&moved, true, true));
    }

    #[test]
    fn unsafe_target_transition_uses_the_full_path() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        assert!(!lifecycle.try_request_steady_state_transition(&settings, true, false));
    }

    #[test]
    fn hide_and_failed_native_operations_invalidate_fast_path_acknowledgements() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        lifecycle.request_hide();
        assert!(lifecycle.applied_settings.is_none());
        assert!(lifecycle.shown_target_is_live.is_none());
        assert!(!lifecycle.try_request_steady_state_transition(&settings, true, true));

        let apply_id = lifecycle.begin_settings_apply();
        lifecycle.acknowledge_settings_apply(apply_id, &settings);
        let pending = lifecycle
            .request_show_after_apply(true)
            .expect("the renderer remains ready after hiding");
        assert!(lifecycle.begin_show(pending));
        // No acknowledgement models a failed native show.
        assert!(!lifecycle.try_request_steady_state_transition(&settings, false, true));

        let apply_id = lifecycle.begin_settings_apply();
        assert!(lifecycle.applied_settings.is_none());
        // No acknowledgement models a partial or failed native apply.
        assert_ne!(apply_id, 0);
        assert!(!lifecycle.try_request_steady_state_transition(&settings, true, true));
    }

    #[test]
    fn stale_native_acknowledgements_cannot_restore_fast_path_state() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        let stale_apply = lifecycle.begin_settings_apply();
        let stale_show = lifecycle
            .request_show_after_apply(false)
            .expect("the renderer is ready");
        assert!(lifecycle.begin_show(stale_show));
        lifecycle.request_hide();
        lifecycle.acknowledge_settings_apply(stale_apply, &settings);
        lifecycle.acknowledge_show(stale_show);

        assert!(lifecycle.applied_settings.is_none());
        assert!(lifecycle.shown_target_is_live.is_none());
    }

    #[test]
    fn create_reset_rejects_acknowledgements_from_the_previous_window() {
        let mut lifecycle = HudLifecycleState::default();
        let settings = HudSettings::default();
        settle_visible_hud(&mut lifecycle, &settings, false);

        let stale_apply = lifecycle.begin_settings_apply();
        let stale_show = lifecycle
            .request_show_after_apply(false)
            .expect("the renderer is ready");
        assert!(lifecycle.begin_show(stale_show));
        lifecycle.reset_for_create();
        lifecycle.acknowledge_settings_apply(stale_apply, &settings);
        lifecycle.acknowledge_show(stale_show);

        assert!(!lifecycle.renderer_ready);
        assert!(!lifecycle.show_requested);
        assert!(lifecycle.applied_settings.is_none());
        assert!(lifecycle.shown_target_is_live.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_drag_requires_a_left_mouse_down_from_the_panel() {
        assert!(is_native_drag_event(NS_EVENT_TYPE_LEFT_MOUSE_DOWN, 42, 42));
        assert!(!is_native_drag_event(2, 42, 42));
        assert!(!is_native_drag_event(NS_EVENT_TYPE_LEFT_MOUSE_DOWN, 41, 42));
        assert!(!is_native_drag_event(NS_EVENT_TYPE_LEFT_MOUSE_DOWN, 0, 0));
    }

    #[test]
    fn computes_bottom_center_inside_the_work_area() {
        assert_eq!(
            preset_coordinates(
                monitor((0, 0, 1440, 900), (0, 24, 1440, 876), 1.0),
                HudPresentation::Expanded,
                HudPosition::BottomCenter,
            ),
            (552, 780)
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
            (-360, 960)
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
            (6000, 1920)
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
            (1368, 766)
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
            Some(HudCoordinates { x: -336, y: 984 })
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
            Some(HudCoordinates { x: 2928, y: 1744 })
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
            Some(HudCoordinates { x: 1104, y: 300 })
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
            (48, 110)
        );
        assert_eq!(
            physical_dimensions(HudPresentation::Expanded, 0.0),
            (336, 96)
        );
    }
}
