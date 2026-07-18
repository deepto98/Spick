mod audio;
mod commands;
pub mod domain;
pub mod engines;
mod hud;
mod model_store;
pub mod platform;
mod session;
mod shortcut;
mod state;

use std::io;

use tauri::Manager;

use crate::state::AppState;

const MAIN_WINDOW_LABEL: &str = "main";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // Keep settings writes and global shortcut ownership process-local. The
    // single-instance plugin must be registered before every other plugin.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        reveal_main_window(app);
    }));

    let app = builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    shortcut::handle_event(app, event.state());
                })
                .build(),
        )
        .on_window_event(|window, event| {
            if window.label() == MAIN_WINDOW_LABEL {
                #[cfg(target_os = "macos")]
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Spick remains available to the global shortcut after the
                    // dashboard closes. Dock reopen or a second launch reveals
                    // this same window, while the app menu still provides Quit.
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        eprintln!("could not hide the Spick dashboard: {error}");
                    }
                }

                #[cfg(not(target_os = "macos"))]
                if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                    // Until Spick has a tray menu with an explicit Quit action,
                    // closing its only user-facing window must end the process.
                    window.app_handle().exit(0);
                }
            }
        })
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let models_path = app.path().app_local_data_dir()?.join("models");
            let state =
                AppState::load_with_models(settings_path, models_path).map_err(setup_error)?;
            let settings = state.settings_snapshot().map_err(setup_error)?;

            if !app.manage(state) {
                return Err(setup_error("application state was already initialized"));
            }
            preload_active_model(app.handle().clone());

            // A missing monitor or an unavailable global binding should not make
            // the settings window unusable. Both can be corrected after launch.
            if let Err(error) = hud::create(app.handle(), settings.hud.position) {
                eprintln!("{error}");
            }
            if platform::current_platform_capabilities().supports_global_shortcut {
                if let Err(error) =
                    shortcut::register(app.handle(), &settings.push_to_talk_shortcut)
                {
                    eprintln!("{error}");
                }
            } else {
                eprintln!("global shortcuts are unavailable in this desktop session");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::get_dictation_session,
            commands::get_audio_capture_status,
            commands::get_last_transcript,
            commands::list_local_models,
            commands::install_local_model,
            commands::cancel_local_model_install,
            commands::activate_local_model,
            commands::remove_local_model,
            commands::start_dictation_session,
            commands::stop_dictation_session,
            commands::cancel_dictation_session,
            commands::get_platform_capabilities,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Spick");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            reveal_main_window(app);
        }

        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}

fn preload_active_model<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    let spawn_result = std::thread::Builder::new()
        .name("spick-model-preload".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            let Ok(settings) = state.settings_snapshot() else {
                return;
            };
            if settings.transcription_engine.provider != domain::EngineProvider::WhisperCpp
                || settings.transcription_engine.location != domain::EngineLocation::Local
            {
                return;
            }
            let Some(model) =
                engines::resolve_curated_whisper_model(&settings.transcription_engine.model)
            else {
                return;
            };
            let should_load = state
                .models
                .catalog(&model.id)
                .into_iter()
                .find(|summary| summary.manifest.id == model.id)
                .is_some_and(|summary| {
                    matches!(
                        summary.state,
                        model_store::ModelInstallationState::Installed
                            | model_store::ModelInstallationState::NeedsVerification
                    )
                });
            if !should_load {
                return;
            }

            let result = state
                .models
                .verified_model_path(&model.id)
                .map_err(engines::EngineError::Backend)
                .and_then(|path| state.whisper.load(&model, &path));
            if let Err(error) = result {
                eprintln!("could not preload the active local model: {error}");
            }
        });
    if let Err(error) = spawn_result {
        eprintln!("could not start local model preloading: {error}");
    }
}

fn reveal_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn setup_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(io::Error::other(message.into()))
}
