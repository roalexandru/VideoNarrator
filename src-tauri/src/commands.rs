//! Tauri command handlers for the Narrator application.

use crate::error::NarratorError;
use crate::models::*;
use crate::{
    ai_client, doc_processor, elevenlabs_client, export_engine, project_store, screen_recorder,
    video_edit, video_engine,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::sync::Mutex;

// ── Persistent config file for API keys ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct PersistentConfig {
    #[serde(default)]
    api_keys: std::collections::HashMap<String, String>,
    #[serde(default)]
    elevenlabs: Option<ElevenLabsPersisted>,
    /// Whether anonymous telemetry is enabled. Defaults to true when missing (first launch).
    #[serde(default)]
    telemetry_enabled: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ElevenLabsPersisted {
    api_key: String,
    voice_id: String,
    model_id: String,
    stability: f32,
    similarity_boost: f32,
    style: f32,
    speed: f32,
}

fn config_path() -> PathBuf {
    project_store::get_narrator_dir().join("config.json")
}

fn load_config() -> PersistentConfig {
    let path = config_path();
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        PersistentConfig::default()
    }
}

fn save_config(config: &PersistentConfig) -> Result<(), NarratorError> {
    let path = config_path();
    let json = serde_json::to_string_pretty(config)?;
    // Write atomically with restrictive permissions from the start
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(json.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, json)?;
    }
    Ok(())
}

// ── App state ──

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
    pub api_keys: Mutex<std::collections::HashMap<AiProviderKind, String>>,
}

impl AppState {
    pub fn new() -> Self {
        // Load persisted API keys on startup
        let config = load_config();
        let mut keys = std::collections::HashMap::new();
        for (k, v) in &config.api_keys {
            match k.as_str() {
                "claude" => {
                    keys.insert(AiProviderKind::Claude, v.clone());
                }
                "openai" => {
                    keys.insert(AiProviderKind::OpenAi, v.clone());
                }
                _ => {}
            }
        }
        tracing::info!("Loaded {} persisted API keys", keys.len());

        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            api_keys: Mutex::new(keys),
        }
    }
}

// ── Telemetry commands ──

#[tauri::command]
pub fn get_telemetry_enabled() -> bool {
    load_config().telemetry_enabled.unwrap_or(true)
}

#[tauri::command]
pub fn set_telemetry_enabled(enabled: bool) -> Result<(), NarratorError> {
    let mut config = load_config();
    config.telemetry_enabled = Some(enabled);
    save_config(&config)
}

#[tauri::command]
pub fn track_event(
    name: String,
    props: Option<serde_json::Value>,
    state: tauri::State<'_, std::sync::Arc<crate::telemetry::TelemetryClient>>,
) {
    // Opt-out is enforced by the frontend (analytics.ts) before calling this command.
    // No disk read here — the frontend holds the in-memory telemetry flag.
    state.track(name, props);
}

// ── System commands ──

#[tauri::command]
pub async fn check_ffmpeg() -> Result<String, NarratorError> {
    let path = video_engine::detect_ffmpeg()?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_provider_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderKeyStatus>, NarratorError> {
    let keys = state.api_keys.lock().await;
    Ok(vec![
        ProviderKeyStatus {
            provider: AiProviderKind::Claude,
            has_key: keys.contains_key(&AiProviderKind::Claude),
            models: ai_client::get_available_models(&AiProviderKind::Claude),
        },
        ProviderKeyStatus {
            provider: AiProviderKind::OpenAi,
            has_key: keys.contains_key(&AiProviderKind::OpenAi),
            models: ai_client::get_available_models(&AiProviderKind::OpenAi),
        },
    ])
}

#[tauri::command]
pub async fn set_api_key(
    state: tauri::State<'_, AppState>,
    provider: AiProviderKind,
    key: String,
) -> Result<(), NarratorError> {
    // Update in-memory
    let mut keys = state.api_keys.lock().await;
    keys.insert(provider.clone(), key.clone());
    drop(keys);

    // Persist to disk
    let mut config = load_config();
    config.api_keys.insert(provider.to_string(), key);
    save_config(&config)?;

    Ok(())
}

#[tauri::command]
pub async fn validate_api_key_cmd(
    provider: AiProviderKind,
    key: String,
) -> Result<bool, NarratorError> {
    ai_client::validate_api_key(&provider, &key).await
}

// ── Video commands ──

#[tauri::command]
pub async fn probe_video(path: String) -> Result<VideoMetadata, NarratorError> {
    if !Path::new(&path).exists() {
        return Err(NarratorError::VideoProbeError("File not found".to_string()));
    }
    video_engine::probe_video(Path::new(&path)).await
}

// ── Document commands ──

#[tauri::command]
pub async fn process_documents(
    paths: Vec<String>,
) -> Result<Vec<ProcessedDocument>, NarratorError> {
    let mut docs = Vec::new();
    for path in paths {
        let doc = doc_processor::process_document(Path::new(&path))?;
        docs.push(doc);
    }
    Ok(docs)
}

// ── Generation commands ──

#[tauri::command]
pub async fn generate_narration(
    state: tauri::State<'_, AppState>,
    params: GenerationParams,
    channel: Channel<ProgressEvent>,
) -> Result<NarrationScript, NarratorError> {
    // Input validation
    if params.title.len() > 500 {
        return Err(NarratorError::ApiError(
            "Title must be 500 characters or fewer".to_string(),
        ));
    }
    if params.description.len() > 5000 {
        return Err(NarratorError::ApiError(
            "Description must be 5000 characters or fewer".to_string(),
        ));
    }
    if params.custom_prompt.len() > 10000 {
        return Err(NarratorError::ApiError(
            "Custom prompt must be 10000 characters or fewer".to_string(),
        ));
    }

    state.cancel_flag.store(false, Ordering::SeqCst);

    let keys = state.api_keys.lock().await;
    let api_key = keys
        .get(&params.ai_config.provider)
        .ok_or_else(|| NarratorError::NoApiKey(params.ai_config.provider.to_string()))?
        .clone();
    drop(keys);

    let provider = ai_client::create_provider(&params.ai_config, api_key);

    // Phase 1: Extract frames
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "extracting_frames".to_string(),
        })
        .ok();

    // Reuse project_id if provided, otherwise create a new one
    let project_id = if params.project_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        params.project_id.clone()
    };
    let frames_dir = project_store::get_project_frames_dir(&project_id);
    std::fs::create_dir_all(&frames_dir)
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let channel_clone = channel.clone();
    let frames_dir_cleanup = frames_dir.clone();
    let frames = match video_engine::extract_frames(
        Path::new(&params.video_path),
        &params.frame_config,
        &frames_dir,
        move |frame| {
            channel_clone
                .send(ProgressEvent::FrameExtracted {
                    frame: frame.clone(),
                })
                .ok();
        },
    )
    .await
    {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&frames_dir_cleanup);
            return Err(e);
        }
    };

    if state.cancel_flag.load(Ordering::SeqCst) {
        let _ = std::fs::remove_dir_all(&frames_dir);
        return Err(NarratorError::Cancelled);
    }

    // Phase 2: Process documents
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "processing_docs".to_string(),
        })
        .ok();

    let mut docs = Vec::new();
    for path in &params.document_paths {
        match doc_processor::process_document(Path::new(path)) {
            Ok(doc) => docs.push(doc),
            Err(e) => {
                channel
                    .send(ProgressEvent::Error {
                        message: format!("Warning: {e}"),
                    })
                    .ok();
            }
        }
    }
    let docs = doc_processor::truncate_to_budget(docs, 50000);

    if state.cancel_flag.load(Ordering::SeqCst) {
        return Err(NarratorError::Cancelled);
    }

    // Phase 3: Generate narration
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "generating_narration".to_string(),
        })
        .ok();

    let style = project_store::load_styles()?
        .into_iter()
        .find(|s| s.id == params.style)
        .unwrap_or_else(|| project_store::default_styles()[1].clone());

    let video_metadata = video_engine::probe_video(Path::new(&params.video_path)).await?;
    let system_prompt = ai_client::build_system_prompt(&style, &docs, &params.custom_prompt);
    let user_message = ai_client::build_user_message(
        &frames,
        &params.title,
        &params.description,
        &video_metadata,
        &params.primary_language,
    )?;

    let script = ai_client::generate_narration(
        provider.as_ref(),
        &system_prompt,
        user_message,
        &params.style,
        &params.primary_language,
    )
    .await?;

    for segment in &script.segments {
        channel
            .send(ProgressEvent::SegmentStreamed {
                segment: segment.clone(),
            })
            .ok();
    }
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "done".to_string(),
        })
        .ok();

    // Auto-save project
    let project_config = ProjectConfig {
        id: project_id.clone(),
        title: params.title.clone(),
        description: params.description.clone(),
        video_path: params.video_path.clone(),
        style: params.style.clone(),
        languages: {
            let mut l = vec![params.primary_language.clone()];
            l.extend(params.additional_languages.clone());
            l
        },
        primary_language: params.primary_language.clone(),
        frame_config: params.frame_config.clone(),
        ai_config: params.ai_config.clone(),
        custom_prompt: params.custom_prompt.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    let _ = project_store::create_project(&project_config);
    let _ = project_store::save_script(&project_id, &params.primary_language, &script);

    Ok(script)
}

#[tauri::command]
pub async fn translate_script(
    state: tauri::State<'_, AppState>,
    script: NarrationScript,
    target_lang: String,
    ai_config: AiConfig,
) -> Result<NarrationScript, NarratorError> {
    let keys = state.api_keys.lock().await;
    let api_key = keys
        .get(&ai_config.provider)
        .ok_or_else(|| NarratorError::NoApiKey(ai_config.provider.to_string()))?
        .clone();
    drop(keys);
    let provider = ai_client::create_provider(&ai_config, api_key);
    ai_client::translate_script(provider.as_ref(), &script, &target_lang).await
}

#[tauri::command]
pub async fn cancel_generation(state: tauri::State<'_, AppState>) -> Result<(), NarratorError> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    Ok(())
}

// ── Project commands ──

#[tauri::command]
pub async fn save_project(config: ProjectConfig) -> Result<String, NarratorError> {
    project_store::create_project(&config)
}

#[tauri::command]
pub async fn load_project(id: String) -> Result<ProjectConfig, NarratorError> {
    project_store::load_project(&id)
}

#[tauri::command]
pub async fn load_project_full(id: String) -> Result<LoadedProject, NarratorError> {
    project_store::load_project_full(&id)
}

#[tauri::command]
pub async fn list_projects() -> Result<Vec<ProjectSummary>, NarratorError> {
    project_store::list_projects()
}

#[tauri::command]
pub async fn delete_project(id: String) -> Result<(), NarratorError> {
    let dir = project_store::get_narrator_dir().join("projects").join(&id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| NarratorError::ProjectError(format!("Failed to delete project: {e}")))?;
    }
    Ok(())
}

// ── ElevenLabs commands ──

#[tauri::command]
pub async fn get_elevenlabs_config(
) -> Result<Option<elevenlabs_client::ElevenLabsConfig>, NarratorError> {
    let config = load_config();
    Ok(config
        .elevenlabs
        .map(|e| elevenlabs_client::ElevenLabsConfig {
            api_key: e.api_key,
            voice_id: e.voice_id,
            model_id: e.model_id,
            stability: e.stability,
            similarity_boost: e.similarity_boost,
            style: e.style,
            speed: e.speed,
        }))
}

#[tauri::command]
pub async fn save_elevenlabs_config(
    config: elevenlabs_client::ElevenLabsConfig,
) -> Result<(), NarratorError> {
    let mut persistent = load_config();
    persistent.elevenlabs = Some(ElevenLabsPersisted {
        api_key: config.api_key,
        voice_id: config.voice_id,
        model_id: config.model_id,
        stability: config.stability,
        similarity_boost: config.similarity_boost,
        style: config.style,
        speed: config.speed,
    });
    save_config(&persistent)
}

#[tauri::command]
pub async fn list_elevenlabs_voices(
    api_key: String,
) -> Result<Vec<elevenlabs_client::ElevenLabsVoice>, NarratorError> {
    elevenlabs_client::list_voices(&api_key).await
}

#[tauri::command]
pub async fn validate_elevenlabs_key(api_key: String) -> Result<bool, NarratorError> {
    elevenlabs_client::validate_key(&api_key).await
}

#[tauri::command]
pub async fn generate_tts(
    segments: Vec<Segment>,
    output_dir: String,
    compact: bool,
    channel: Channel<ProgressEvent>,
) -> Result<Vec<elevenlabs_client::TtsResult>, NarratorError> {
    let config = load_config();
    let el_config = config
        .elevenlabs
        .map(|e| elevenlabs_client::ElevenLabsConfig {
            api_key: e.api_key,
            voice_id: e.voice_id,
            model_id: e.model_id,
            stability: e.stability,
            similarity_boost: e.similarity_boost,
            style: e.style,
            speed: e.speed,
        })
        .ok_or_else(|| NarratorError::NoApiKey("elevenlabs".to_string()))?;

    let out = PathBuf::from(&output_dir);
    std::fs::create_dir_all(&out)?;

    let mut results = Vec::new();

    if compact {
        // Single file mode: generate per-segment audio, then concatenate with
        // silence gaps matching the video timing so the audio duration matches.
        let total = segments.len();

        // Generate each segment individually
        let mut segment_files: Vec<(usize, PathBuf, f64, f64)> = Vec::new(); // (index, path, start, end)
        for (i, seg) in segments.iter().enumerate() {
            channel
                .send(ProgressEvent::Progress {
                    percent: (i as f64 / total as f64) * 80.0,
                })
                .ok();

            let filename = format!("_tmp_seg_{:03}.mp3", seg.index);
            let filepath = out.join(&filename);

            match elevenlabs_client::generate_speech(&el_config, &seg.text, &filepath).await {
                Ok(()) => {
                    segment_files.push((seg.index, filepath, seg.start_seconds, seg.end_seconds));
                }
                Err(e) => {
                    results.push(elevenlabs_client::TtsResult {
                        segment_index: seg.index,
                        file_path: filepath.to_string_lossy().to_string(),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        channel.send(ProgressEvent::Progress { percent: 85.0 }).ok();

        // Merge using concat demuxer: silence gaps + segments in sequence
        // This preserves full volume for each segment (no amix normalization)
        if !segment_files.is_empty() {
            let final_path = out.join("narration_full.mp3");
            let ffmpeg = video_engine::detect_ffmpeg().unwrap_or_else(|_| PathBuf::from("ffmpeg"));
            let video_dur = segments.last().map(|s| s.end_seconds).unwrap_or(60.0);

            tracing::info!(
                "Concat-merging {} segments into {:.0}s audio",
                segment_files.len(),
                video_dur
            );

            // Build ordered list of files: silence gaps interleaved with segments
            let mut concat_parts: Vec<PathBuf> = Vec::new();
            let mut silence_idx = 0;

            for (i, (_seg_idx, seg_path, start, _end)) in segment_files.iter().enumerate() {
                // Gap before this segment
                let prev_end = if i == 0 { 0.0 } else { segment_files[i - 1].3 };
                let gap = start - prev_end;

                if gap > 0.05 {
                    // Create a silence file for this gap
                    let sil_path = out.join(format!("_tmp_sil_{}.mp3", silence_idx));
                    let _ = tokio::process::Command::new(ffmpeg.as_os_str())
                        .args([
                            "-y",
                            "-f",
                            "lavfi",
                            "-i",
                            "anullsrc=r=44100:cl=stereo",
                            "-t",
                            &format!("{:.3}", gap),
                            "-codec:a",
                            "libmp3lame",
                            "-q:a",
                            "2",
                        ])
                        .arg(sil_path.as_os_str())
                        .output()
                        .await;
                    concat_parts.push(sil_path);
                    silence_idx += 1;
                }

                // The segment audio itself
                concat_parts.push(seg_path.clone());
            }

            // Trailing silence to reach full video duration
            let last_end = segment_files.last().map(|s| s.3).unwrap_or(0.0);
            if video_dur > last_end + 0.1 {
                let trail = video_dur - last_end;
                let sil_path = out.join(format!("_tmp_sil_{}.mp3", silence_idx));
                let _ = tokio::process::Command::new(ffmpeg.as_os_str())
                    .args([
                        "-y",
                        "-f",
                        "lavfi",
                        "-i",
                        "anullsrc=r=44100:cl=stereo",
                        "-t",
                        &format!("{:.3}", trail),
                        "-codec:a",
                        "libmp3lame",
                        "-q:a",
                        "2",
                    ])
                    .arg(sil_path.as_os_str())
                    .output()
                    .await;
                concat_parts.push(sil_path);
            }

            // Write concat list file
            let concat_list_path = out.join("_concat_list.txt");
            let concat_content: String = concat_parts
                .iter()
                .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&concat_list_path, &concat_content)?;

            channel.send(ProgressEvent::Progress { percent: 92.0 }).ok();

            // Run concat
            let concat_output = tokio::process::Command::new(ffmpeg.as_os_str())
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(concat_list_path.as_os_str())
                .args(["-codec:a", "libmp3lame", "-q:a", "2"])
                .arg(final_path.as_os_str())
                .output()
                .await;

            match concat_output {
                Ok(o) if o.status.success() => {
                    results.push(elevenlabs_client::TtsResult {
                        segment_index: 0,
                        file_path: final_path.to_string_lossy().to_string(),
                        success: true,
                        error: None,
                    });
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    tracing::error!("ffmpeg concat failed: {}", &stderr[..stderr.len().min(500)]);
                    for (idx, path, _, _) in &segment_files {
                        results.push(elevenlabs_client::TtsResult {
                            segment_index: *idx,
                            file_path: path.to_string_lossy().to_string(),
                            success: true,
                            error: Some("Concat failed — kept individual file".into()),
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("ffmpeg exec failed: {e}");
                    for (idx, path, _, _) in &segment_files {
                        results.push(elevenlabs_client::TtsResult {
                            segment_index: *idx,
                            file_path: path.to_string_lossy().to_string(),
                            success: true,
                            error: Some("Concat failed — kept individual file".into()),
                        });
                    }
                }
            }

            // Clean up temp files
            let _ = std::fs::remove_file(&concat_list_path);
            for part in &concat_parts {
                if part.to_string_lossy().contains("_tmp_sil_") {
                    let _ = std::fs::remove_file(part);
                }
            }
            for (_, path, _, _) in &segment_files {
                let _ = std::fs::remove_file(path);
            }
        }
    } else {
        // Per-segment mode
        let total = segments.len();
        for (i, seg) in segments.iter().enumerate() {
            channel
                .send(ProgressEvent::Progress {
                    percent: (i as f64 / total as f64) * 100.0,
                })
                .ok();

            let filename = format!("segment_{:03}.mp3", seg.index);
            let filepath = out.join(&filename);

            match elevenlabs_client::generate_speech(&el_config, &seg.text, &filepath).await {
                Ok(()) => {
                    results.push(elevenlabs_client::TtsResult {
                        segment_index: seg.index,
                        file_path: filepath.to_string_lossy().to_string(),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    results.push(elevenlabs_client::TtsResult {
                        segment_index: seg.index,
                        file_path: filepath.to_string_lossy().to_string(),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    }

    channel
        .send(ProgressEvent::Progress { percent: 100.0 })
        .ok();
    Ok(results)
}

#[tauri::command]
pub async fn get_home_dir() -> Result<String, NarratorError> {
    let dir = project_store::get_narrator_dir();
    // Return parent of .narrator (= home dir)
    Ok(dir.parent().unwrap_or(&dir).to_string_lossy().to_string())
}

// ── Screen recording commands ──

/// Native screen recording: opens macOS Cmd+Shift+5 UI, blocks until done
#[tauri::command]
pub async fn record_screen_native(output_path: String) -> Result<String, NarratorError> {
    screen_recorder::record_native(&output_path).await
}

/// Legacy ffmpeg recording (Windows fallback)
#[tauri::command]
pub async fn start_recording(
    state: tauri::State<'_, AppState>,
    config: screen_recorder::RecordingConfig,
) -> Result<String, NarratorError> {
    state.cancel_flag.store(false, Ordering::SeqCst);
    let flag = state.cancel_flag.clone();
    screen_recorder::start_recording(&config, flag).await
}

#[tauri::command]
pub async fn stop_recording(state: tauri::State<'_, AppState>) -> Result<(), NarratorError> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}

// ── Video edit commands ──

#[tauri::command]
pub async fn apply_video_edits(
    input_path: String,
    output_path: String,
    edits: video_edit::VideoEditPlan,
    channel: Channel<ProgressEvent>,
) -> Result<String, NarratorError> {
    video_edit::apply_edits(&input_path, &output_path, &edits, |pct| {
        channel.send(ProgressEvent::Progress { percent: pct }).ok();
    })
    .await
}

#[tauri::command]
pub async fn extract_edit_thumbnails(
    video_path: String,
    output_dir: String,
    count: usize,
) -> Result<Vec<String>, NarratorError> {
    video_edit::extract_edit_thumbnails(&video_path, &output_dir, count).await
}

#[tauri::command]
pub async fn merge_audio_video(
    video_path: String,
    audio_path: String,
    output_path: String,
    replace_audio: bool,
) -> Result<String, NarratorError> {
    video_edit::merge_audio_video(&video_path, &audio_path, &output_path, replace_audio).await
}

#[tauri::command]
pub async fn open_folder(path: String) -> Result<(), NarratorError> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    }
    Ok(())
}

// ── Frame commands ──

#[tauri::command]
pub async fn list_project_frames(project_id: String) -> Result<Vec<ProjectFrame>, NarratorError> {
    let frames_dir = project_store::get_project_frames_dir(&project_id);
    if !frames_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&frames_dir)
        .map_err(|e| NarratorError::ProjectError(e.to_string()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "jpg" || x == "jpeg" || x == "png")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut frames = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let path = entry.path().to_string_lossy().to_string();
        // Parse timestamp from filename pattern frame_NNNN.jpg → index * interval
        // We don't know the exact interval, but frames are sequential
        frames.push(ProjectFrame { index: i, path });
    }
    Ok(frames)
}

// ── Export commands ──

#[tauri::command]
pub async fn export_script(options: ExportOptions) -> Result<Vec<ExportResult>, NarratorError> {
    let mut results = Vec::new();
    let output_dir = PathBuf::from(&options.output_directory);
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| NarratorError::ExportError(format!("Failed to create output dir: {e}")))?;

    for language in &options.languages {
        if let Some(script) = options.scripts.get(language) {
            for format in &options.formats {
                let content = match format {
                    ExportFormat::Json => export_engine::export_json(script),
                    ExportFormat::Srt => export_engine::export_srt(script),
                    ExportFormat::Vtt => export_engine::export_vtt(script),
                    ExportFormat::Txt => export_engine::export_txt(script),
                    ExportFormat::Markdown => export_engine::export_markdown(script),
                    ExportFormat::Ssml => export_engine::export_ssml(script),
                };
                let basename = options.basename.as_deref().unwrap_or("narration");
                let filename = if options.languages.len() > 1 {
                    format!("{basename}_{language}.{format}")
                } else {
                    format!("{basename}.{format}")
                };
                let filepath = output_dir.join(&filename);
                match std::fs::write(&filepath, &content) {
                    Ok(()) => results.push(ExportResult {
                        format: format.to_string(),
                        language: language.clone(),
                        file_path: filepath.to_string_lossy().to_string(),
                        success: true,
                        error: None,
                    }),
                    Err(e) => results.push(ExportResult {
                        format: format.to_string(),
                        language: language.clone(),
                        file_path: filepath.to_string_lossy().to_string(),
                        success: false,
                        error: Some(e.to_string()),
                    }),
                }
            }
        }
    }
    Ok(results)
}

// ── Subtitle burn command ──

#[tauri::command]
pub async fn burn_subtitles(
    video_path: String,
    srt_content: String,
    output_path: String,
) -> Result<String, NarratorError> {
    // Write SRT to temp file, then burn
    let out_dir = std::path::Path::new(&output_path)
        .parent()
        .unwrap_or(std::path::Path::new("/tmp"));
    let srt_path = out_dir.join("_temp_subtitles.srt");
    std::fs::write(&srt_path, &srt_content)?;

    let result =
        video_edit::burn_subtitles(&video_path, &srt_path.to_string_lossy(), &output_path).await;

    let _ = std::fs::remove_file(&srt_path);
    result
}

// ── Style commands ──

#[tauri::command]
pub async fn list_styles() -> Result<Vec<NarrationStyle>, NarratorError> {
    project_store::load_styles()
}
