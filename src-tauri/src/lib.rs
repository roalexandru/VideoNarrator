//! Narrator - AI-powered video narration generator. Tauri application entry point.

mod ai_client;
mod azure_tts_client;
mod builtin_tts;
mod commands;
mod doc_processor;
mod elevenlabs_client;
mod error;
mod export_engine;
mod http_client;
mod menu;
mod models;
mod process_utils;
mod project_store;
mod screen_recorder;
mod secure_store;
mod telemetry;
mod video_edit;
mod video_engine;

use commands::AppState;
use tauri::Emitter;

/// Called from the frontend whenever the view changes so we can
/// enable/disable project-dependent menu items (macOS native menu only).
#[tauri::command]
fn set_menu_context(app: tauri::AppHandle, has_project: bool) {
    #[cfg(target_os = "macos")]
    menu::set_project_context(&app, has_project);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, has_project);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    // Ensure ~/.narrator/ directories exist
    if let Err(e) = project_store::ensure_directories() {
        tracing::warn!("Failed to create narrator directories: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // When a second instance tries to launch, focus the existing window
            use tauri::Manager;
            let windows = app.webview_windows();
            if let Some(w) = windows.values().next() {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|_app| {
            // Only use native menu on macOS (it supports dark mode).
            // On Windows/Linux, a custom webview menu bar is used instead.
            #[cfg(target_os = "macos")]
            {
                let m = menu::build(_app)?;
                _app.set_menu(m)?;
                menu::set_native_help_menu();
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id.starts_with(menu::RECENT_PREFIX) {
                // "recent:<project_id>" → emit with the full id so frontend can extract it
                let _ = app.emit("menu-event", id);
            } else {
                match id {
                    menu::NEW_PROJECT
                    | menu::OPEN_PROJECT
                    | menu::SAVE_PROJECT
                    | menu::OPEN_SETTINGS
                    | menu::NARRATOR_HELP
                    | menu::SEND_FEEDBACK
                    | menu::ABOUT_NARRATOR
                    | menu::CHECK_FOR_UPDATES
                    | "toggle_fullscreen" => {
                        let _ = app.emit("menu-event", id);
                    }
                    _ => {}
                }
            }
        })
        .manage(AppState::new())
        .manage(commands::RecorderState::new())
        .manage(telemetry::TelemetryClient::new(
            env!("CARGO_PKG_VERSION").into(),
        ))
        .invoke_handler(tauri::generate_handler![
            set_menu_context,
            commands::check_ffmpeg,
            commands::get_provider_status,
            commands::set_api_key,
            commands::validate_api_key_cmd,
            commands::probe_video,
            commands::check_file_readable,
            commands::file_exists,
            commands::process_documents,
            commands::generate_narration,
            commands::translate_script,
            commands::refine_segment,
            commands::refine_script,
            commands::cancel_generation,
            commands::save_project,
            commands::load_project,
            commands::load_project_full,
            commands::list_projects,
            commands::delete_project,
            commands::export_project,
            commands::import_project,
            commands::save_template,
            commands::list_templates,
            commands::delete_template,
            commands::get_elevenlabs_config,
            commands::save_elevenlabs_config,
            commands::list_elevenlabs_voices,
            commands::validate_elevenlabs_key,
            commands::get_azure_tts_config,
            commands::save_azure_tts_config,
            commands::list_azure_tts_voices,
            commands::validate_azure_tts_key,
            commands::get_tts_provider,
            commands::save_tts_provider,
            commands::generate_tts,
            commands::get_home_dir,
            commands::record_screen_native,
            commands::start_screen_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::stop_screen_recording,
            commands::get_recordings_directory,
            commands::apply_video_edits,
            commands::extract_edit_thumbnails,
            commands::extract_single_frame,
            commands::save_script,
            commands::merge_audio_video,
            commands::open_folder,
            commands::list_project_frames,
            commands::export_script,
            commands::burn_subtitles,
            commands::list_styles,
            commands::get_telemetry_enabled,
            commands::set_telemetry_enabled,
            commands::track_event,
            commands::list_builtin_voices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
