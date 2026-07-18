use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder,
};

use crate::domain::HudPosition;

pub const HUD_WINDOW_LABEL: &str = "hud";
const HUD_WIDTH: f64 = 360.0;
const HUD_HEIGHT: f64 = 104.0;
const HUD_MARGIN: f64 = 24.0;

pub fn create<R: Runtime>(
    app: &AppHandle<R>,
    preferred_position: HudPosition,
) -> Result<(), String> {
    if app.get_webview_window(HUD_WINDOW_LABEL).is_some() {
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        HUD_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=hud".into()),
    )
    .title("Spick Dictation")
    .inner_size(HUD_WIDTH, HUD_HEIGHT)
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

    if let Some(position) = position_for_target_monitor(app, preferred_position)? {
        window
            .set_position(position)
            .map_err(|error| format!("could not position dictation HUD: {error}"))?;
    }

    Ok(())
}

pub fn reposition<R: Runtime>(
    app: &AppHandle<R>,
    preferred_position: HudPosition,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) else {
        return Err("dictation HUD is not available".into());
    };
    let Some(position) = position_for_target_monitor(app, preferred_position)? else {
        return Ok(());
    };

    window
        .set_position(position)
        .map_err(|error| format!("could not position dictation HUD: {error}"))
}

pub fn show<R: Runtime>(app: &AppHandle<R>, preferred_position: HudPosition) -> Result<(), String> {
    // Monitor enumeration can transiently fail while a display is attached or
    // removed. Keep the last valid location and still provide capture feedback.
    if let Err(error) = reposition(app, preferred_position) {
        eprintln!("showing the dictation HUD at its last position: {error}");
    }
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    window
        .show()
        .map_err(|error| format!("could not show dictation HUD: {error}"))
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "dictation HUD is not available".to_string())?;
    window
        .hide()
        .map_err(|error| format!("could not hide dictation HUD: {error}"))
}

fn position_for_target_monitor<R: Runtime>(
    app: &AppHandle<R>,
    preferred_position: HudPosition,
) -> Result<Option<PhysicalPosition<i32>>, String> {
    let cursor = app.cursor_position().ok();
    let monitors = app
        .available_monitors()
        .map_err(|error| format!("could not inspect available monitors: {error}"))?;
    let monitor = cursor.and_then(|cursor| {
        monitors.into_iter().find(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            contains_physical_point(
                cursor.x,
                cursor.y,
                position.x,
                position.y,
                size.width,
                size.height,
            )
        })
    });
    let monitor = match monitor {
        Some(monitor) => Some(monitor),
        None => app
            .primary_monitor()
            .map_err(|error| format!("could not inspect the primary monitor: {error}"))?,
    };
    let Some(monitor) = monitor else {
        return Ok(None);
    };

    let work_area = monitor.work_area();
    let (x, y) = coordinates(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width,
        work_area.size.height,
        monitor.scale_factor(),
        preferred_position,
    );

    Ok(Some(PhysicalPosition::new(x, y)))
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

fn coordinates(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
    preferred_position: HudPosition,
) -> (i32, i32) {
    let work_x = f64::from(work_x);
    let work_y = f64::from(work_y);
    let work_width = f64::from(work_width);
    let work_height = f64::from(work_height);
    let hud_width = HUD_WIDTH * scale_factor;
    let hud_height = HUD_HEIGHT * scale_factor;
    let margin = HUD_MARGIN * scale_factor;
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

    #[test]
    fn computes_bottom_center_inside_the_work_area() {
        assert_eq!(
            coordinates(0, 24, 1440, 876, 1.0, HudPosition::BottomCenter),
            (540, 772)
        );
    }

    #[test]
    fn preserves_negative_monitor_origins() {
        assert_eq!(
            coordinates(-1920, 0, 1920, 1080, 1.0, HudPosition::BottomRight),
            (-384, 952)
        );
    }

    #[test]
    fn uses_the_target_monitor_scale_for_mixed_dpi_layouts() {
        assert_eq!(
            coordinates(2880, 0, 3840, 2160, 2.0, HudPosition::BottomRight),
            (5952, 1904)
        );
    }

    #[test]
    fn identifies_the_monitor_under_a_cursor_with_negative_origins() {
        assert!(contains_physical_point(
            -1200.0, 400.0, -1920, 0, 1920, 1080
        ));
        assert!(!contains_physical_point(20.0, 400.0, -1920, 0, 1920, 1080));
    }
}
