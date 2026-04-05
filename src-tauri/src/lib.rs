//! Narrator - AI-powered video narration generator. Tauri application entry point.

mod ai_client;
mod commands;
mod doc_processor;
mod elevenlabs_client;
mod error;
mod export_engine;
mod menu;
mod models;
mod project_store;
mod screen_recorder;
mod video_edit;
mod video_engine;

use commands::AppState;
use tauri::Emitter;

/// Called from the frontend whenever the view changes so we can
/// enable/disable project-dependent menu items.
#[tauri::command]
fn set_menu_context(app: tauri::AppHandle, has_project: bool) {
    menu::set_project_context(&app, has_project);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    // Ensure ~/.narrator/ directories exist
    if let Err(e) = project_store::ensure_directories() {
        tracing::warn!("Failed to create narrator directories: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let m = menu::build(app)?;
            app.set_menu(m)?;
            #[cfg(target_os = "macos")]
            menu::set_native_help_menu();
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
                    | "toggle_fullscreen" => {
                        let _ = app.emit("menu-event", id);
                    }
                    _ => {}
                }
            }
        })
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            set_menu_context,
            commands::check_ffmpeg,
            commands::get_provider_status,
            commands::set_api_key,
            commands::validate_api_key_cmd,
            commands::probe_video,
            commands::process_documents,
            commands::generate_narration,
            commands::translate_script,
            commands::cancel_generation,
            commands::save_project,
            commands::load_project,
            commands::load_project_full,
            commands::list_projects,
            commands::delete_project,
            commands::get_elevenlabs_config,
            commands::save_elevenlabs_config,
            commands::list_elevenlabs_voices,
            commands::validate_elevenlabs_key,
            commands::generate_tts,
            commands::get_home_dir,
            commands::record_screen_native,
            commands::start_recording,
            commands::stop_recording,
            commands::apply_video_edits,
            commands::extract_edit_thumbnails,
            commands::merge_audio_video,
            commands::open_folder,
            commands::list_project_frames,
            commands::export_script,
            commands::burn_subtitles,
            commands::list_styles,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
