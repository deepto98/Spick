use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl, WebviewWindowBuilder,
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

    let mut builder = WebviewWindowBuilder::new(
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
    .prevent_overflow_with_margin(LogicalSize::new(HUD_MARGIN, HUD_MARGIN));

    if let Some((x, y)) = coordinates_for_primary_monitor(app, preferred_position)? {
        builder = builder.position(x, y);
    }

    builder
        .build()
        .map(|_| ())
        .map_err(|error| format!("could not create dictation HUD: {error}"))
}

pub fn reposition<R: Runtime>(
    app: &AppHandle<R>,
    preferred_position: HudPosition,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(HUD_WINDOW_LABEL) else {
        return Err("dictation HUD is not available".into());
    };
    let Some((x, y)) = coordinates_for_primary_monitor(app, preferred_position)? else {
        return Ok(());
    };

    window
        .set_position(LogicalPosition::new(x, y))
        .map_err(|error| format!("could not position dictation HUD: {error}"))
}

pub fn show<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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

fn coordinates_for_primary_monitor<R: Runtime>(
    app: &AppHandle<R>,
    preferred_position: HudPosition,
) -> Result<Option<(f64, f64)>, String> {
    let monitor = app
        .primary_monitor()
        .map_err(|error| format!("could not inspect the primary monitor: {error}"))?;
    let Some(monitor) = monitor else {
        return Ok(None);
    };

    let work_area = monitor.work_area();
    let scale = monitor.scale_factor();
    let logical_x = f64::from(work_area.position.x) / scale;
    let logical_y = f64::from(work_area.position.y) / scale;
    let logical_width = f64::from(work_area.size.width) / scale;
    let logical_height = f64::from(work_area.size.height) / scale;

    Ok(Some(coordinates(
        logical_x,
        logical_y,
        logical_width,
        logical_height,
        preferred_position,
    )))
}

fn coordinates(
    work_x: f64,
    work_y: f64,
    work_width: f64,
    work_height: f64,
    preferred_position: HudPosition,
) -> (f64, f64) {
    let x = match preferred_position {
        HudPosition::BottomLeft => work_x + HUD_MARGIN,
        HudPosition::BottomCenter => work_x + (work_width - HUD_WIDTH) / 2.0,
        HudPosition::BottomRight => work_x + work_width - HUD_WIDTH - HUD_MARGIN,
    };
    let y = work_y + work_height - HUD_HEIGHT - HUD_MARGIN;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_bottom_center_inside_the_work_area() {
        assert_eq!(
            coordinates(0.0, 24.0, 1440.0, 876.0, HudPosition::BottomCenter),
            (540.0, 772.0)
        );
    }

    #[test]
    fn preserves_negative_monitor_origins() {
        assert_eq!(
            coordinates(-1920.0, 0.0, 1920.0, 1080.0, HudPosition::BottomRight),
            (-384.0, 952.0)
        );
    }
}
