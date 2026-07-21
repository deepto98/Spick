#[cfg(all(feature = "macos-input-method-unsafe-dev-peers", not(debug_assertions)))]
compile_error!("macos-input-method-unsafe-dev-peers is forbidden in release builds");
#[cfg(all(
    feature = "macos-input-method-compatibility-harness",
    not(debug_assertions)
))]
compile_error!("macos-input-method-compatibility-harness is forbidden in release builds");

mod audio;
mod cloud;
mod commands;
#[cfg(all(
    target_os = "macos",
    feature = "macos-input-method-compatibility-harness"
))]
pub mod compatibility;
pub mod domain;
pub mod engines;
mod hud;
mod latency;
mod local_data;
mod microphone_permission;
mod model_store;
mod notes;
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
    let builder = if compatibility_mode_active() {
        // A compatibility run is a single, explicit diagnostic process. Do
        // not accept unauthenticated forwarded argv over the single-instance
        // plugin's local socket.
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal_main_window(app);
        }))
    };

    // The dictation HUD is converted once to a nonactivating NSPanel. Register
    // the native handle store before setup creates the HUD window.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    let app = builder
        .plugin(tauri_plugin_dialog::init())
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
            #[cfg(all(
                target_os = "macos",
                feature = "macos-input-method-compatibility-harness"
            ))]
            if compatibility::is_active() {
                // The generated context has no windows in this mode, so no
                // production IPC command can be invoked.
                shortcut::register(app.handle(), compatibility::COMPATIBILITY_SHORTCUT)
                    .map_err(setup_error)?;
                compatibility::start_watchdog(app.handle().clone()).map_err(setup_error)?;
                eprintln!("{}", compatibility::ready_message());
                return Ok(());
            }

            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let local_data_dir = app.path().app_local_data_dir()?;
            let models_path = local_data_dir.join("models");
            let database_path = local_data_dir.join("spick.sqlite3");
            let state = AppState::load_with_paths(settings_path, models_path, database_path)
                .map_err(setup_error)?;
            if let Some(reason) = state.local_data.unavailable_reason() {
                // Dictation remains usable: only optional statistics, saved
                // history, and vocabulary persistence are disabled.
                eprintln!("{reason}");
            }
            let settings = state.settings_snapshot().map_err(setup_error)?;

            if settings.transcription_engine.model == "whisper-tiny-multilingual-f16" {
                let resource_dir = app.path().resource_dir()?;
                let bundled_model = [
                    resource_dir.join("resources/models/ggml-tiny.bin"),
                    resource_dir.join("models/ggml-tiny.bin"),
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("resources/models/ggml-tiny.bin"),
                ]
                .into_iter()
                .find(|path| path.is_file())
                .ok_or_else(|| setup_error("the bundled Whisper Tiny model is missing"))?;
                state
                    .models
                    .seed_bundled_model("whisper-tiny-multilingual-f16", &bundled_model)
                    .map_err(setup_error)?;
            }

            if !app.manage(state) {
                return Err(setup_error("application state was already initialized"));
            }
            preload_active_model(app.handle().clone());

            // A missing monitor or an unavailable global binding should not make
            // the settings window unusable. Both can be corrected after launch.
            match hud::create(app.handle(), &settings.hud) {
                Ok(()) if settings.hud.visible => {
                    // `show` records the desired visibility now, but the native
                    // window remains hidden until its renderer acknowledges that
                    // the persisted presentation has been committed.
                    if let Err(error) = hud::show(app.handle(), &settings.hud, false) {
                        eprintln!("could not show the saved floating widget: {error}");
                    }
                }
                Ok(()) => {}
                Err(error) => eprintln!("{error}"),
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
            cloud::list_cloud_providers,
            cloud::set_cloud_api_key,
            cloud::delete_cloud_api_key,
            cloud::activate_cloud_provider,
            commands::get_usage_dashboard,
            commands::list_transcript_history,
            commands::list_vocabulary,
            commands::create_vocabulary_entry,
            commands::update_vocabulary_entry,
            commands::delete_vocabulary_entry,
            commands::list_notes,
            commands::create_note,
            commands::update_note,
            commands::delete_note,
            commands::export_note,
            commands::clear_local_data,
            commands::get_dictation_session,
            commands::get_audio_capture_status,
            commands::list_audio_input_devices,
            commands::get_shortcut_status,
            commands::request_input_monitoring_permission,
            commands::get_hud_settings,
            commands::mark_hud_renderer_ready,
            commands::set_hud_presentation,
            commands::start_hud_drag,
            commands::get_last_transcript,
            commands::get_last_dictation_latency,
            commands::list_local_models,
            commands::import_local_model,
            commands::install_local_model,
            commands::cancel_local_model_install,
            commands::activate_local_model,
            commands::remove_local_model,
            commands::start_dictation_session,
            commands::set_in_app_dictation_mode,
            commands::stop_dictation_session,
            commands::cancel_dictation_session,
            commands::get_platform_capabilities,
            commands::get_accessibility_permission_status,
            commands::request_accessibility_permission,
            commands::get_microphone_permission_status,
            commands::request_microphone_permission,
        ])
        .build({
            let mut context = tauri::generate_context!();
            if compatibility_mode_active() {
                context.config_mut().app.windows.clear();
            }
            context
        })
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

fn compatibility_mode_active() -> bool {
    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    {
        compatibility::is_active()
    }

    #[cfg(not(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    )))]
    {
        false
    }
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
            let Some(model) = state.models.resolve(&settings.transcription_engine.model) else {
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
