mod ai_client;
mod commands;
mod doc_processor;
mod elevenlabs_client;
mod error;
mod export_engine;
mod models;
mod project_store;
mod screen_recorder;
mod video_edit;
mod video_engine;

use commands::AppState;

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
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
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
            commands::open_recorder_window,
            commands::close_recorder_window,
            commands::list_screens,
            commands::list_windows,
            commands::start_recording,
            commands::stop_recording,
            commands::apply_video_edits,
            commands::extract_edit_thumbnails,
            commands::merge_audio_video,
            commands::open_folder,
            commands::list_project_frames,
            commands::export_script,
            commands::list_styles,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
