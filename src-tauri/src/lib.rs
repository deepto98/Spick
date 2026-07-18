mod audio;
mod commands;
pub mod domain;
pub mod engines;
mod hud;
pub mod platform;
mod session;
mod shortcut;
mod state;

use std::io;

use tauri::Manager;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    shortcut::handle_event(app, event.state());
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let state = AppState::load(settings_path).map_err(setup_error)?;
            let settings = state.settings_snapshot().map_err(setup_error)?;

            if !app.manage(state) {
                return Err(setup_error("application state was already initialized"));
            }

            // A missing monitor or an unavailable global binding should not make
            // the settings window unusable. Both can be corrected after launch.
            if let Err(error) = hud::create(app.handle(), settings.hud.position) {
                eprintln!("{error}");
            }
            if let Err(error) = shortcut::register(app.handle(), &settings.push_to_talk_shortcut) {
                eprintln!("{error}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::get_dictation_session,
            commands::get_audio_capture_status,
            commands::start_dictation_session,
            commands::stop_dictation_session,
            commands::cancel_dictation_session,
            commands::get_platform_capabilities,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Spick");
}

fn setup_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(io::Error::other(message.into()))
}
