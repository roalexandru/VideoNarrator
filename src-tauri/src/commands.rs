//! Tauri command handlers for the Narrator application.

use crate::error::NarratorError;
use crate::models::*;
use crate::render;
use crate::secure_store;
use crate::{
    ai_client, azure_tts_client, builtin_tts, doc_processor, elevenlabs_client, export_engine,
    project_store, screen_recorder, video_edit, video_engine,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

static LAST_VALIDATION: AtomicI64 = AtomicI64::new(0);
use tauri::ipc::Channel;
use tauri::Manager;
use tokio::sync::Mutex;

/// Strip the `\\?\` extended-length path prefix that Windows `canonicalize()` adds.
fn strip_extended_path_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

// ── Persistent config file for API keys ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct PersistentConfig {
    #[serde(default)]
    api_keys: std::collections::HashMap<String, String>,
    #[serde(default)]
    elevenlabs: Option<ElevenLabsPersisted>,
    #[serde(default)]
    azure_tts: Option<AzureTtsPersisted>,
    /// Persisted TTS provider preference ("elevenlabs" or "azure").
    #[serde(default)]
    tts_provider: Option<String>,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AzureTtsPersisted {
    api_key: String,
    region: String,
    voice_name: String,
    speaking_style: String,
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
    // Write atomically via temp file + rename. Set 0o600 on the temp file
    // BEFORE the rename so the final file is never briefly world-readable.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let parent = path
            .parent()
            .ok_or_else(|| NarratorError::ProjectError("config path has no parent".into()))?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("config.json");
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp = parent.join(format!(".{file_name}.tmp.{nonce}"));
        let write_result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, &path)
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        write_result?;
    }
    #[cfg(not(unix))]
    {
        crate::project_store::atomic_write(&path, json.as_bytes())?;
    }
    Ok(())
}

// ── Secure API key helpers ──

/// Try to save an API key to OS keychain. Returns true if successful.
fn save_api_key_secure(provider: &str, key: &str) -> bool {
    let keychain_key = format!("api_key_{provider}");
    match secure_store::set_secret(&keychain_key, key) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(
                "Keychain write failed for {provider}, will use config.json fallback: {e}"
            );
            false
        }
    }
}

fn load_api_key_secure(provider: &str) -> Option<String> {
    let keychain_key = format!("api_key_{provider}");
    secure_store::get_secret(&keychain_key).ok().flatten()
}

// ── App state ──

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
    pub api_keys: Mutex<std::collections::HashMap<AiProviderKind, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecordingPhase {
    Idle,
    Recording,
    Paused,
    Stopping,
}

pub struct RecorderState {
    pub ffmpeg_child: Mutex<Option<tokio::process::Child>>,
    pub segments: Mutex<Vec<String>>,
    pub phase: Mutex<RecordingPhase>,
    pub output_dir: Mutex<Option<String>>,
    pub output_path: Mutex<Option<String>>,
    pub segment_counter: std::sync::atomic::AtomicU32,
}

impl RecorderState {
    pub fn new() -> Self {
        Self {
            ffmpeg_child: Mutex::new(None),
            segments: Mutex::new(Vec::new()),
            phase: Mutex::new(RecordingPhase::Idle),
            output_dir: Mutex::new(None),
            output_path: Mutex::new(None),
            segment_counter: std::sync::atomic::AtomicU32::new(0),
        }
    }

    async fn reset(&self) {
        *self.ffmpeg_child.lock().await = None;
        self.segments.lock().await.clear();
        *self.phase.lock().await = RecordingPhase::Idle;
        *self.output_dir.lock().await = None;
        *self.output_path.lock().await = None;
        self.segment_counter
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

impl AppState {
    pub fn new() -> Self {
        let config = load_config();
        let mut keys = std::collections::HashMap::new();

        // Migrate old per-key keychain entries into single bundled entry
        // (reduces macOS keychain prompts from N to 1)
        secure_store::migrate_old_keychain_entries();

        // Migrate plaintext keys to keychain if they exist
        if !config.api_keys.is_empty() {
            let migrated = secure_store::migrate_from_plaintext(&config.api_keys);
            if migrated > 0 {
                // Remove keys from the JSON file now that they're in the keychain
                let mut clean_config = config.clone();
                clean_config.api_keys.clear();
                if let Err(e) = save_config(&clean_config) {
                    tracing::warn!("Failed to clean API keys from config.json: {e}");
                }
                tracing::info!("Migrated {migrated} API keys to OS keychain");
            }
        }

        // Migrate ElevenLabs API key from config to keychain
        if let Some(ref el) = config.elevenlabs {
            if !el.api_key.is_empty() && save_api_key_secure("elevenlabs", &el.api_key) {
                let mut clean_config = load_config();
                if let Some(ref mut el_cfg) = clean_config.elevenlabs {
                    el_cfg.api_key = String::new();
                }
                if let Err(e) = save_config(&clean_config) {
                    tracing::warn!("Failed to clean ElevenLabs key from config.json: {e}");
                }
                tracing::info!("Migrated ElevenLabs API key to OS keychain");
            }
        }

        // Migrate Azure TTS API key from config to keychain
        if let Some(ref az) = config.azure_tts {
            if !az.api_key.is_empty() && save_api_key_secure("azure_tts", &az.api_key) {
                let mut clean_config = load_config();
                if let Some(ref mut az_cfg) = clean_config.azure_tts {
                    az_cfg.api_key = String::new();
                }
                if let Err(e) = save_config(&clean_config) {
                    tracing::warn!("Failed to clean Azure TTS key from config.json: {e}");
                }
                tracing::info!("Migrated Azure TTS API key to OS keychain");
            }
        }

        // Load AI provider keys from keychain
        for (provider_str, provider_kind) in [
            ("claude", AiProviderKind::Claude),
            ("openai", AiProviderKind::OpenAi),
            ("gemini", AiProviderKind::Gemini),
        ] {
            if let Some(key) = load_api_key_secure(provider_str) {
                keys.insert(provider_kind, key);
            }
        }

        tracing::info!("Loaded {} API keys from secure storage", keys.len());

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
        ProviderKeyStatus {
            provider: AiProviderKind::Gemini,
            has_key: keys.contains_key(&AiProviderKind::Gemini),
            models: ai_client::get_available_models(&AiProviderKind::Gemini),
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

    // Persist: try OS keychain first, fall back to config.json
    let provider_str = provider.to_string();
    if !save_api_key_secure(&provider_str, &key) {
        // Keychain unavailable — persist to config.json
        let mut config = load_config();
        config.api_keys.insert(provider_str, key);
        save_config(&config)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn validate_api_key_cmd(
    provider: AiProviderKind,
    key: String,
) -> Result<bool, NarratorError> {
    // Rate limit: max one validation per 2 seconds
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let last = LAST_VALIDATION.load(Ordering::SeqCst);
    if now - last < 2 {
        return Err(NarratorError::ApiError(
            "Please wait before validating again".to_string(),
        ));
    }
    LAST_VALIDATION.store(now, Ordering::SeqCst);
    ai_client::validate_api_key(&provider, &key).await
}

// ── Video commands ──

#[tauri::command]
pub async fn probe_video(path: String) -> Result<VideoMetadata, NarratorError> {
    let path = Path::new(&path);
    if !path.exists() {
        return Err(NarratorError::VideoProbeError("File not found".to_string()));
    }
    // Try to canonicalize (resolves symlinks, prevents traversal), but fall back
    // to the original path if canonicalization fails (e.g., sandboxed macOS apps)
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    // Strip \\?\ extended-length path prefix that Windows canonicalize adds
    let resolved = strip_extended_path_prefix(&resolved);
    video_engine::probe_video(&resolved).await
}

/// Return whether a file exists at the given path. Used by Export to detect
/// a missing cached edited video and trigger regeneration.
#[tauri::command]
pub fn file_exists(path: String) -> bool {
    std::path::Path::new(&path).is_file()
}

/// Check whether the app can actually read the given file path. On macOS,
/// this surfaces TCC denials (Documents/Desktop/Downloads folder permissions)
/// as a clear error instead of a silent black video preview.
#[tauri::command]
pub async fn check_file_readable(path: String) -> Result<bool, NarratorError> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(NarratorError::VideoProbeError(format!(
            "File not found: {path}"
        )));
    }
    // Attempt an actual read of the first byte. Just checking metadata is
    // insufficient because macOS TCC allows stat() but blocks read().
    match tokio::fs::File::open(p).await {
        Ok(mut f) => {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1];
            match f.read(&mut buf).await {
                Ok(_) => Ok(true),
                Err(e) if e.raw_os_error() == Some(1) => {
                    // errno 1 = EPERM on macOS → TCC denial
                    Err(NarratorError::VideoProbeError(
                        "macOS has denied access to this file. Go to System Settings → \
                         Privacy & Security → Files and Folders, and enable access for Narrator \
                         (Documents / Desktop / Downloads as applicable). Or move the file to \
                         ~/Downloads or ~/Videos which don't need special permission."
                            .into(),
                    ))
                }
                Err(e) => Err(NarratorError::VideoProbeError(format!(
                    "Cannot read file: {e}"
                ))),
            }
        }
        Err(e) if e.raw_os_error() == Some(1) => Err(NarratorError::VideoProbeError(
            "macOS has denied access to this file. Go to System Settings → \
             Privacy & Security → Files and Folders, and enable access for Narrator \
             (Documents / Desktop / Downloads as applicable). Or move the file to \
             ~/Downloads or ~/Videos which don't need special permission."
                .into(),
        )),
        Err(e) => Err(NarratorError::VideoProbeError(format!(
            "Cannot open file: {e}"
        ))),
    }
}

// ── Document commands ──

#[tauri::command]
pub async fn process_documents(
    paths: Vec<String>,
) -> Result<Vec<ProcessedDocument>, NarratorError> {
    if paths.len() > 20 {
        return Err(NarratorError::DocumentError(
            "Maximum 20 documents allowed".to_string(),
        ));
    }
    let mut docs = Vec::new();
    let mut total_size: usize = 0;
    const MAX_TOTAL_SIZE: usize = 10 * 1024 * 1024; // 10MB
    for path in paths {
        let p = Path::new(&path);
        if !p.exists() {
            return Err(NarratorError::DocumentError(format!(
                "File not found: {path}"
            )));
        }
        let resolved = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let resolved = strip_extended_path_prefix(&resolved);
        let doc = doc_processor::process_document(&resolved)?;
        total_size += doc.content.len();
        if total_size > MAX_TOTAL_SIZE {
            return Err(NarratorError::DocumentError(
                "Total document size exceeds 10MB limit".to_string(),
            ));
        }
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
    // Validate frame config bounds
    let mut frame_config = params.frame_config.clone();
    if frame_config.max_frames > 100 {
        tracing::warn!(
            "max_frames {} exceeds limit, capping to 100",
            frame_config.max_frames
        );
        frame_config.max_frames = 100;
    }

    let frames_dir = project_store::get_project_frames_dir(&project_id);
    tokio::fs::create_dir_all(&frames_dir)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let channel_for_frames = channel.clone();
    let channel_for_ticks = channel.clone();
    let frames_dir_cleanup = frames_dir.clone();
    let frames = match video_engine::extract_frames(
        Path::new(&params.video_path),
        &frame_config,
        &frames_dir,
        move |frame| {
            channel_for_frames
                .send(ProgressEvent::FrameExtracted {
                    frame: frame.clone(),
                })
                .ok();
        },
        move |fraction, message| {
            // Frame extraction owns 0..25% of the global bar.
            let percent = (fraction.clamp(0.0, 1.0)) * 25.0;
            channel_for_ticks
                .send(ProgressEvent::progress_msg(percent, message))
                .ok();
        },
    )
    .await
    {
        Ok(f) => f,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&frames_dir_cleanup).await;
            return Err(e);
        }
    };

    if state.cancel_flag.load(Ordering::SeqCst) {
        let _ = tokio::fs::remove_dir_all(&frames_dir).await;
        return Err(NarratorError::Cancelled);
    }

    // Phase 2: Process documents — the 25%→30% slice of the global bar.
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "processing_docs".to_string(),
        })
        .ok();

    let total_docs = params.document_paths.len();
    let mut docs = Vec::new();
    if total_docs == 0 {
        channel
            .send(ProgressEvent::progress_msg(30.0, "No context documents"))
            .ok();
    } else {
        for (i, path) in params.document_paths.iter().enumerate() {
            // Base of the slice for this document.
            let base = 25.0 + (i as f64 / total_docs as f64) * 5.0;
            let name = Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("document");
            channel
                .send(ProgressEvent::progress_msg(
                    base,
                    format!("Reading {name} ({}/{total_docs})", i + 1),
                ))
                .ok();

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
        channel.send(ProgressEvent::progress(30.0)).ok();
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
    let system_prompt = ai_client::build_system_prompt(
        &style,
        &docs,
        &params.custom_prompt,
        &params.primary_language,
    );
    // build_user_message reads frame files from disk and base64-encodes them (CPU+IO bound)
    let frames_clone = frames.clone();
    let title = params.title.clone();
    let description = params.description.clone();
    let vm = video_metadata.clone();
    let lang = params.primary_language.clone();
    let user_message = tokio::task::spawn_blocking(move || {
        ai_client::build_user_message(&frames_clone, &title, &description, &vm, &lang)
    })
    .await
    .map_err(|e| NarratorError::ApiError(e.to_string()))??;

    // Pre-count image parts so we know which path `generate_narration` will
    // take. Chunked path drives real per-chunk progress ticks; single-call
    // path has none, so we fall back to a smooth logarithmic timer. Running
    // both would race, so they're mutually exclusive.
    let image_count = user_message
        .as_array()
        .map(|p| p.iter().filter(|v| v["type"] == "image").count())
        .unwrap_or(0);
    let will_chunk = image_count > ai_client::MAX_FRAMES_PER_CALL;

    // Narration owns 30% → 99% of the global bar. Both callback branches
    // (chunked per-chunk ticks AND the single-call logarithmic fallback) map
    // their local 0..1 / 0..95 range into that slice via the same helper so
    // the frontend just reads `percent` — no phase-aware math.
    const NARRATION_BASE: f64 = 30.0;
    const NARRATION_SPAN: f64 = 69.0;
    let scale_narration =
        |fraction: f64| NARRATION_BASE + fraction.clamp(0.0, 1.0) * NARRATION_SPAN;

    // Logarithmic progress reporter: ONLY runs on the single-call path
    // (no chunking ⇒ nothing else to report ticks). On the chunked path the
    // per-chunk callback is the source of truth and the timer is never
    // spawned (would race with real progress and walk the bar backward on
    // monotonic clamp).
    let progress_task_handle = if !will_chunk {
        let progress_channel = channel.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();
        let handle = tokio::spawn(async move {
            let mut local_pct = 0.0_f64;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if cancel_clone.load(Ordering::SeqCst) {
                    break;
                }
                // Logarithmic progress: approaches but never reaches 95% of
                // the narration slice.
                local_pct += (95.0 - local_pct) * 0.15;
                let percent = NARRATION_BASE + (local_pct / 100.0) * NARRATION_SPAN;
                progress_channel.send(ProgressEvent::progress(percent)).ok();
            }
        });
        Some((handle, cancel_flag))
    } else {
        None
    };

    // Stream per-chunk segments to the frontend as generate_chunked produces
    // them. Primary "live" preview source during chunked generation, and the
    // segments the frontend sends back via `resume_segments` on retry to skip
    // already-completed chunks.
    let stream_channel = channel.clone();
    let on_segment: ai_client::SegmentCallback = Arc::new(move |seg: &Segment| {
        stream_channel
            .send(ProgressEvent::SegmentStreamed {
                segment: seg.clone(),
            })
            .ok();
    });

    // Per-chunk percent + label. The chunked path calls this at the start
    // and end of every chunk (plus once on resume to jump the bar forward);
    // the single-call path calls it once at ~5% with a kickoff label.
    let progress_ch = channel.clone();
    let on_progress_cb: ai_client::ProgressCallback =
        Arc::new(move |fraction: f64, message: Option<String>| {
            let percent = scale_narration(fraction);
            let event = match message {
                Some(msg) => ProgressEvent::progress_msg(percent, msg),
                None => ProgressEvent::progress(percent),
            };
            progress_ch.send(event).ok();
        });

    let script = ai_client::generate_narration(
        provider.as_ref(),
        &system_prompt,
        user_message,
        &params.style,
        &params.primary_language,
        params.resume_segments.clone(),
        Some(on_segment),
        Some(on_progress_cb),
    )
    .await;

    // Stop estimated progress reporter (single-call path only).
    if let Some((handle, cancel_flag)) = progress_task_handle {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Err(e) = handle.await {
            tracing::warn!("Progress reporter task failed: {e}");
        }
    }

    let script = script?;

    // Finalize: pin the bar to 100% with a one-word label so the UI shows
    // a clean endcap before the terminal events. Must fire BEFORE
    // SegmentsReplaced so the frontend's monotonic clamp is at 100 when it
    // transitions to "done".
    channel
        .send(ProgressEvent::progress_msg(100.0, "Complete"))
        .ok();

    // Terminal event: replace the live per-chunk preview with the final,
    // normalized/polished script so what the user sees matches what's saved.
    // Without this the preview could show pre-polish drafts + leftover entries.
    channel
        .send(ProgressEvent::SegmentsReplaced {
            segments: script.segments.clone(),
        })
        .ok();
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "done".to_string(),
        })
        .ok();

    // Auto-save project — preserve existing edits and original video path.
    // Load the current project config first so we don't overwrite edit_clips/effects.
    let existing_config = project_store::load_project(&project_id).ok();
    let original_video_path = existing_config
        .as_ref()
        .map(|c| c.video_path.clone())
        .unwrap_or_else(|| params.video_path.clone());
    let project_config = ProjectConfig {
        schema_version: 1,
        id: project_id.clone(),
        title: params.title.clone(),
        description: params.description.clone(),
        // Always save the ORIGINAL video path, not the edited one
        video_path: original_video_path,
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
        created_at: existing_config
            .as_ref()
            .map(|c| c.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        updated_at: chrono::Utc::now().to_rfc3339(),
        // Preserve existing edits — never wipe them during generation
        edit_clips: existing_config.as_ref().and_then(|c| c.edit_clips.clone()),
        timeline_effects: existing_config
            .as_ref()
            .and_then(|c| c.timeline_effects.clone()),
        video_metadata: existing_config
            .as_ref()
            .and_then(|c| c.video_metadata.clone()),
        // Preserve context documents across regenerations
        context_documents: existing_config
            .as_ref()
            .and_then(|c| c.context_documents.clone()),
        // Preserve the cached-edited-video path + its hash
        edited_video_path: existing_config
            .as_ref()
            .and_then(|c| c.edited_video_path.clone()),
        edited_video_plan_hash: existing_config
            .as_ref()
            .and_then(|c| c.edited_video_plan_hash.clone()),
    };
    if let Err(e) = project_store::create_project(&project_config) {
        tracing::warn!("Failed to auto-save project: {e}");
    }
    if let Err(e) = project_store::save_script(&project_id, &params.primary_language, &script) {
        tracing::warn!("Failed to auto-save script: {e}");
    }

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
pub async fn refine_segment(
    state: tauri::State<'_, AppState>,
    segment_text: String,
    instruction: String,
    context: String,
    ai_config: AiConfig,
) -> Result<String, NarratorError> {
    let keys = state.api_keys.lock().await;
    let api_key = keys
        .get(&ai_config.provider)
        .ok_or_else(|| NarratorError::NoApiKey(ai_config.provider.to_string()))?
        .clone();
    drop(keys);
    let provider = ai_client::create_provider(&ai_config, api_key);
    ai_client::refine_segment(provider.as_ref(), &segment_text, &instruction, &context).await
}

/// Whole-script AI refinement. Rewrites the entire narration to satisfy a
/// user instruction while preserving timeline structure and style.
#[tauri::command]
pub async fn refine_script(
    state: tauri::State<'_, AppState>,
    script: NarrationScript,
    instruction: String,
    ai_config: AiConfig,
    style_hint: Option<String>,
    custom_prompt: Option<String>,
) -> Result<NarrationScript, NarratorError> {
    let keys = state.api_keys.lock().await;
    let api_key = keys
        .get(&ai_config.provider)
        .ok_or_else(|| NarratorError::NoApiKey(ai_config.provider.to_string()))?
        .clone();
    drop(keys);
    let provider = ai_client::create_provider(&ai_config, api_key);
    let style = style_hint.as_deref().unwrap_or("professional narration");
    ai_client::refine_script(
        provider.as_ref(),
        &script,
        &instruction,
        style,
        custom_prompt.as_deref(),
    )
    .await
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
    project_store::validate_project_id(&id)?;
    let dir = project_store::get_narrator_dir().join("projects").join(&id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| NarratorError::ProjectError(format!("Failed to delete project: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn export_project(id: String, output_path: String) -> Result<(), NarratorError> {
    project_store::export_project(&id, std::path::Path::new(&output_path))
}

#[tauri::command]
pub async fn import_project(archive_path: String) -> Result<String, NarratorError> {
    project_store::import_project(std::path::Path::new(&archive_path))
}

// ── Template commands ──

#[tauri::command]
pub async fn save_template(template: ProjectTemplate) -> Result<(), NarratorError> {
    project_store::save_template(&template)
}

#[tauri::command]
pub async fn list_templates() -> Result<Vec<ProjectTemplate>, NarratorError> {
    project_store::list_templates()
}

#[tauri::command]
pub async fn delete_template(id: String) -> Result<(), NarratorError> {
    project_store::delete_template(&id)
}

// ── ElevenLabs commands ──

#[tauri::command]
pub async fn get_elevenlabs_config(
) -> Result<Option<elevenlabs_client::ElevenLabsConfig>, NarratorError> {
    let config = load_config();
    Ok(config.elevenlabs.map(|e| {
        // Inject API key from keychain (config.json stores empty string)
        let api_key = load_api_key_secure("elevenlabs").unwrap_or(e.api_key);
        elevenlabs_client::ElevenLabsConfig {
            api_key,
            voice_id: e.voice_id,
            model_id: e.model_id,
            stability: e.stability,
            similarity_boost: e.similarity_boost,
            style: e.style,
            speed: e.speed,
        }
    }))
}

#[tauri::command]
pub async fn save_elevenlabs_config(
    config: elevenlabs_client::ElevenLabsConfig,
) -> Result<(), NarratorError> {
    // Try OS keychain; if it fails, keep key in JSON
    let keychain_ok = save_api_key_secure("elevenlabs", &config.api_key);

    let mut persistent = load_config();
    persistent.elevenlabs = Some(ElevenLabsPersisted {
        api_key: if keychain_ok {
            String::new()
        } else {
            config.api_key
        },
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

// ── Azure TTS commands ──

#[tauri::command]
pub async fn get_azure_tts_config(
) -> Result<Option<azure_tts_client::AzureTtsConfig>, NarratorError> {
    let config = load_config();
    Ok(config.azure_tts.map(|a| {
        // Inject API key from keychain (config.json stores empty string)
        let api_key = load_api_key_secure("azure_tts").unwrap_or(a.api_key);
        azure_tts_client::AzureTtsConfig {
            api_key,
            region: a.region,
            voice_name: a.voice_name,
            speaking_style: a.speaking_style,
            speed: a.speed,
        }
    }))
}

#[tauri::command]
pub async fn save_azure_tts_config(
    config: azure_tts_client::AzureTtsConfig,
) -> Result<(), NarratorError> {
    // Try OS keychain; if it fails, keep key in JSON
    let keychain_ok = save_api_key_secure("azure_tts", &config.api_key);

    let mut persistent = load_config();
    persistent.azure_tts = Some(AzureTtsPersisted {
        api_key: if keychain_ok {
            String::new()
        } else {
            config.api_key
        },
        region: config.region,
        voice_name: config.voice_name,
        speaking_style: config.speaking_style,
        speed: config.speed,
    });
    save_config(&persistent)
}

#[tauri::command]
pub async fn list_azure_tts_voices(
    api_key: String,
    region: String,
) -> Result<Vec<azure_tts_client::AzureTtsVoice>, NarratorError> {
    azure_tts_client::list_voices(&api_key, &region).await
}

#[tauri::command]
pub async fn validate_azure_tts_key(
    api_key: String,
    region: String,
) -> Result<bool, NarratorError> {
    azure_tts_client::validate_key(&api_key, &region).await
}

// ── Built-in TTS commands ──

#[tauri::command]
pub async fn list_builtin_voices() -> Result<Vec<builtin_tts::BuiltinVoice>, NarratorError> {
    builtin_tts::list_voices().await
}

// ── TTS provider preference ──

#[tauri::command]
pub async fn get_tts_provider() -> Result<Option<String>, NarratorError> {
    let config = load_config();
    // Return persisted preference, or auto-detect from which provider has a key
    if let Some(provider) = config.tts_provider {
        return Ok(Some(provider));
    }
    // Auto-detect: if azure_tts is configured but elevenlabs is not, default to azure
    if config.azure_tts.is_some() && config.elevenlabs.is_none() {
        return Ok(Some("azure".to_string()));
    }
    Ok(None)
}

#[tauri::command]
pub async fn save_tts_provider(provider: String) -> Result<(), NarratorError> {
    let mut config = load_config();
    config.tts_provider = Some(provider);
    save_config(&config)
}

// ── TTS generation command ──

#[derive(Clone)]
enum TtsProvider {
    ElevenLabs(elevenlabs_client::ElevenLabsConfig),
    Azure(azure_tts_client::AzureTtsConfig),
    Builtin { voice: String, speed: f32 },
}

/// Clean text before sending to TTS: remove [pause], [break], (pause), etc.
fn sanitize_tts_text(text: &str) -> String {
    let cleaned = text
        .replace("[pause]", " ")
        .replace("[break]", " ")
        .replace("(pause)", " ")
        .replace("(break)", " ")
        .replace("[silence]", " ")
        .replace("[Pause]", " ")
        .replace("[PAUSE]", " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a cache key from text + provider settings.
fn tts_cache_key(tts: &TtsProvider, text: &str) -> String {
    let settings = match tts {
        TtsProvider::ElevenLabs(c) => format!(
            "el|{}|{}|{:.2}|{:.2}|{:.2}|{:.2}",
            c.voice_id, c.model_id, c.stability, c.similarity_boost, c.style, c.speed
        ),
        TtsProvider::Azure(c) => format!("az|{}|{}|{:.2}", c.voice_name, c.speaking_style, c.speed),
        TtsProvider::Builtin { voice, speed } => format!("builtin|{}|{:.2}", voice, speed),
    };
    let input = format!("{settings}|{text}");
    let hash = blake3::hash(input.as_bytes());
    hash.to_hex().to_string()
}

/// Get the TTS cache directory, creating it if needed.
fn tts_cache_dir() -> Result<PathBuf, NarratorError> {
    let dir = project_store::get_narrator_dir().join("cache").join("tts");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Evict oldest entries from the TTS cache if it exceeds the size limit.
/// Runs opportunistically — errors are silently ignored.
fn evict_tts_cache(cache_dir: &Path) {
    const MAX_CACHE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB

    let entries: Vec<_> = match std::fs::read_dir(cache_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };

    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
        .iter()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if meta.is_file() {
                let modified = meta.modified().ok()?;
                Some((e.path(), meta.len(), modified))
            } else {
                None
            }
        })
        .collect();

    let total_size: u64 = files.iter().map(|(_, s, _)| s).sum();
    if total_size <= MAX_CACHE_SIZE {
        return;
    }

    // Sort oldest first
    files.sort_by_key(|(_, _, t)| *t);

    let mut freed = 0u64;
    let target = total_size - MAX_CACHE_SIZE;
    for (path, size, _) in &files {
        if freed >= target {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            freed += size;
        }
    }
    tracing::info!("TTS cache eviction: freed {} MB", freed / (1024 * 1024));
}

/// Apply a per-segment voice override to a TTS provider.
/// Returns a modified provider with the overridden voice, or clones the original.
fn apply_voice_override(base: &TtsProvider, voice_override: &Option<String>) -> TtsProvider {
    let voice_id = match voice_override {
        Some(v) if !v.is_empty() => v,
        _ => return base.clone(),
    };
    match base {
        TtsProvider::ElevenLabs(cfg) => {
            TtsProvider::ElevenLabs(elevenlabs_client::ElevenLabsConfig {
                voice_id: voice_id.clone(),
                ..cfg.clone()
            })
        }
        TtsProvider::Azure(cfg) => TtsProvider::Azure(azure_tts_client::AzureTtsConfig {
            voice_name: voice_id.clone(),
            ..cfg.clone()
        }),
        TtsProvider::Builtin { speed, .. } => TtsProvider::Builtin {
            voice: voice_id.clone(),
            speed: *speed,
        },
    }
}

/// Generate speech for a single segment, using cache if available.
async fn generate_speech_for_provider(
    tts: &TtsProvider,
    text: &str,
    filepath: &PathBuf,
) -> Result<(), NarratorError> {
    let text = sanitize_tts_text(text);
    let text = text.as_str();

    // Check cache first
    let cache_key = tts_cache_key(tts, text);
    if let Ok(cache_dir) = tts_cache_dir() {
        let cached = cache_dir.join(format!("{cache_key}.mp3"));
        if cached.exists() {
            match tokio::fs::copy(&cached, filepath).await {
                Ok(_) => {
                    tracing::info!("TTS cache hit: {cache_key}");
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("TTS cache read failed, regenerating: {e}");
                    // Fall through to API generation
                }
            }
        }
    }

    // Generate via API
    match tts {
        TtsProvider::ElevenLabs(cfg) => {
            elevenlabs_client::generate_speech(cfg, text, filepath).await?;
        }
        TtsProvider::Azure(cfg) => {
            azure_tts_client::generate_speech(cfg, text, filepath).await?;
        }
        TtsProvider::Builtin { voice, speed } => {
            builtin_tts::generate_speech(text, voice, *speed, filepath).await?;
        }
    }

    // Store in cache and evict old entries if over size limit
    if let Ok(cache_dir) = tts_cache_dir() {
        let cached = cache_dir.join(format!("{cache_key}.mp3"));
        let _ = tokio::fs::copy(filepath, &cached).await;
        let cd = cache_dir.clone();
        tokio::task::spawn_blocking(move || evict_tts_cache(&cd))
            .await
            .ok();
    }

    Ok(())
}

#[tauri::command]
pub async fn generate_tts(
    segments: Vec<Segment>,
    output_dir: String,
    compact: bool,
    tts_provider: Option<String>,
    channel: Channel<ProgressEvent>,
) -> Result<Vec<elevenlabs_client::TtsResult>, NarratorError> {
    let provider_name = tts_provider.unwrap_or_else(|| "elevenlabs".to_string());
    let config = load_config();

    let tts = if provider_name.starts_with("builtin") {
        // Format: "builtin" or "builtin:voice_id:speed"
        let parts: Vec<&str> = provider_name.splitn(3, ':').collect();
        let voice = parts.get(1).unwrap_or(&"default").to_string();
        let speed = parts
            .get(2)
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0);
        TtsProvider::Builtin { voice, speed }
    } else {
        match provider_name.as_str() {
            "azure" => {
                let az_config = config
                    .azure_tts
                    .map(|a| {
                        let api_key = load_api_key_secure("azure_tts").unwrap_or(a.api_key);
                        azure_tts_client::AzureTtsConfig {
                            api_key,
                            region: a.region,
                            voice_name: a.voice_name,
                            speaking_style: a.speaking_style,
                            speed: a.speed,
                        }
                    })
                    .ok_or_else(|| NarratorError::NoApiKey("azure_tts".to_string()))?;
                TtsProvider::Azure(az_config)
            }
            _ => {
                let el_config = config
                    .elevenlabs
                    .map(|e| {
                        let api_key = load_api_key_secure("elevenlabs").unwrap_or(e.api_key);
                        elevenlabs_client::ElevenLabsConfig {
                            api_key,
                            voice_id: e.voice_id,
                            model_id: e.model_id,
                            stability: e.stability,
                            similarity_boost: e.similarity_boost,
                            style: e.style,
                            speed: e.speed,
                        }
                    })
                    .ok_or_else(|| NarratorError::NoApiKey("elevenlabs".to_string()))?;
                TtsProvider::ElevenLabs(el_config)
            }
        }
    };

    let silence_sample_rate = match &tts {
        TtsProvider::Azure(_) => "24000",
        TtsProvider::ElevenLabs(_) => "44100",
        TtsProvider::Builtin { .. } => "44100",
    };

    let out = PathBuf::from(&output_dir);
    tokio::fs::create_dir_all(&out).await?;

    let mut results = Vec::new();

    if compact {
        // Single file mode: generate per-segment audio, then concatenate with
        // silence gaps matching the video timing so the audio duration matches.
        let total = segments.len();

        // Delete any stale timings sidecar from a prior run. If we don't,
        // and today's run fails (or the sidecar write below silently fails),
        // burn_subtitles will happily align today's audio against yesterday's
        // timings — segments appear off by whatever drift last run had.
        let stale_sidecar = out.join("narration_timings.json");
        let _ = tokio::fs::remove_file(&stale_sidecar).await;

        // Generate each segment individually
        let mut segment_files: Vec<(usize, PathBuf, f64, f64)> = Vec::new(); // (index, path, start, end)
        for (i, seg) in segments.iter().enumerate() {
            channel
                .send(ProgressEvent::progress((i as f64 / total as f64) * 80.0))
                .ok();

            let filename = format!("_tmp_seg_{:03}.mp3", seg.index);
            let filepath = out.join(&filename);

            let seg_tts = apply_voice_override(&tts, &seg.voice_override);
            match generate_speech_for_provider(&seg_tts, &seg.text, &filepath).await {
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

        channel.send(ProgressEvent::progress(85.0)).ok();

        // Merge using concat demuxer: silence gaps + segments in sequence
        // This preserves full volume for each segment (no amix normalization)
        if !segment_files.is_empty() {
            let final_path = out.join("narration_full.mp3");
            let video_dur = segments.last().map(|s| s.end_seconds).unwrap_or(60.0);

            channel.send(ProgressEvent::progress(92.0)).ok();

            let pack_segments: Vec<crate::tts_pack::SegmentFile> = segment_files
                .iter()
                .map(|(idx, path, start, end)| crate::tts_pack::SegmentFile {
                    index: *idx,
                    path: path.clone(),
                    start_seconds: *start,
                    end_seconds: *end,
                })
                .collect();

            let concat_output = crate::tts_pack::concat_narration_segments(
                &pack_segments,
                video_dur,
                &final_path,
                &out,
                silence_sample_rate,
            )
            .await;

            let concat_succeeded = concat_output.is_ok();
            match concat_output {
                Ok(stats) => {
                    // Write the narration_timings.json sidecar alongside the
                    // final mp3 so the burn-subtitles pass can align SRT
                    // cues to where the narration actually landed (vs. the
                    // scripted window, which drifts once atempo kicks in or
                    // a segment overruns past the cap). Best-effort — a
                    // failure here just means burn falls back to scripted
                    // timings.
                    let timings_path = out.join("narration_timings.json");
                    match serde_json::to_string(&stats.actual_timings) {
                        Ok(json) => {
                            if let Err(e) = tokio::fs::write(&timings_path, json).await {
                                tracing::warn!(
                                    "Could not write narration_timings.json sidecar: {e}"
                                );
                            }
                        }
                        Err(e) => tracing::warn!(
                            "Could not serialize narration_timings.json sidecar: {e}"
                        ),
                    }

                    results.push(elevenlabs_client::TtsResult {
                        segment_index: 0,
                        file_path: final_path.to_string_lossy().to_string(),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    tracing::error!("ffmpeg concat failed: {e}");
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

            // Clean up temp files. The concat list + silence padding are
            // never referenced outside this function so they're always safe
            // to delete. Per-segment `_tmp_seg_*` files, however, are the
            // fallback results we just handed back to the caller on concat
            // failure — sweeping them here would leave the UI pointing at
            // paths that don't exist. Only delete them on success.
            let _ = tokio::fs::remove_file(out.join("_concat_list.txt")).await;
            if let Ok(mut entries) = tokio::fs::read_dir(&out).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let is_silence = name.starts_with("_tmp_sil_");
                    let is_segment = name.starts_with("_tmp_seg_");
                    if is_silence || (is_segment && concat_succeeded) {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
    } else {
        // Per-segment mode
        let total = segments.len();
        for (i, seg) in segments.iter().enumerate() {
            channel
                .send(ProgressEvent::progress((i as f64 / total as f64) * 100.0))
                .ok();

            let filename = format!("segment_{:03}.mp3", seg.index);
            let filepath = out.join(&filename);

            let seg_tts = apply_voice_override(&tts, &seg.voice_override);
            match generate_speech_for_provider(&seg_tts, &seg.text, &filepath).await {
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

    channel.send(ProgressEvent::progress(100.0)).ok();
    Ok(results)
}

#[tauri::command]
pub async fn get_home_dir() -> Result<String, NarratorError> {
    let dir = project_store::get_narrator_dir();
    // Return parent of .narrator (= home dir)
    Ok(dir.parent().unwrap_or(&dir).to_string_lossy().to_string())
}

// ── Screen recording commands ──

/// macOS: native Cmd+Shift+5 screen recording. Blocks until the user stops recording.
#[tauri::command]
pub async fn record_screen_native(project_id: String) -> Result<String, NarratorError> {
    project_store::validate_project_id(&project_id)?;
    let dir = screen_recorder::get_recordings_dir()?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let output_path = dir.join(format!("{project_id}_{timestamp}.mov"));
    screen_recorder::record_native(&output_path.to_string_lossy()).await
}

/// Windows: start a screen recording session with an overlay control window.
#[tauri::command]
pub async fn start_screen_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, RecorderState>,
    project_id: String,
) -> Result<(), NarratorError> {
    project_store::validate_project_id(&project_id)?;

    // Reset any previous state
    state.reset().await;

    let dir = screen_recorder::get_recordings_dir()?;
    let out_dir = dir.to_string_lossy().to_string();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let final_path = dir
        .join(format!("{project_id}_{timestamp}.mp4"))
        .to_string_lossy()
        .to_string();

    *state.output_dir.lock().await = Some(out_dir.clone());
    *state.output_path.lock().await = Some(final_path);

    // Create the overlay window FIRST so users see immediate feedback
    #[cfg(target_os = "windows")]
    {
        use tauri::WebviewWindowBuilder;

        let overlay = WebviewWindowBuilder::new(
            &app,
            "recorder",
            tauri::WebviewUrl::App("/recorder-overlay.html".into()),
        )
        .title("Narrator Recording")
        .inner_size(260.0, 72.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(true)
        .theme(Some(tauri::Theme::Dark))
        .build()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to create overlay: {e}")))?;

        // Position at top-center of screen
        if let Ok(Some(monitor)) = overlay.current_monitor() {
            let screen_width = monitor.size().width as f64 / monitor.scale_factor();
            let x = (screen_width - 260.0) / 2.0;
            let _ = overlay.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                x, 20.0,
            )));
        }

        // Try to exclude overlay from screen capture (non-fatal if HWND not ready yet)
        match overlay.hwnd() {
            Ok(hwnd) => screen_recorder::set_window_display_affinity(hwnd.0 as isize),
            Err(e) => tracing::warn!("Could not set display affinity (HWND not ready): {e}"),
        }
    }

    // On non-Windows, just log (shouldn't reach here normally)
    #[cfg(not(target_os = "windows"))]
    {
        let _ = &app;
        tracing::warn!("start_screen_recording called on non-Windows platform");
    }

    // Start the first recording segment (FFmpeg gdigrab init takes ~1-2s)
    let (child, segment_path) = screen_recorder::start_segment(&out_dir, 0, 30).await?;
    *state.ffmpeg_child.lock().await = Some(child);
    state.segments.lock().await.push(segment_path);
    state
        .segment_counter
        .store(1, std::sync::atomic::Ordering::SeqCst);
    *state.phase.lock().await = RecordingPhase::Recording;

    // Notify the overlay that recording has actually started
    {
        use tauri::Emitter;
        let _ = app.emit("recording-started", ());
    }

    Ok(())
}

/// Pause the current recording (stops current segment, new one starts on resume).
#[tauri::command]
pub async fn pause_recording(state: tauri::State<'_, RecorderState>) -> Result<(), NarratorError> {
    let mut phase = state.phase.lock().await;
    if *phase != RecordingPhase::Recording {
        return Ok(());
    }
    *phase = RecordingPhase::Paused;
    drop(phase);

    // Stop the current segment
    let mut child_guard = state.ffmpeg_child.lock().await;
    if let Some(child) = child_guard.as_mut() {
        screen_recorder::stop_segment(child).await?;
    }
    *child_guard = None;

    tracing::info!("Recording paused");
    Ok(())
}

/// Resume a paused recording (starts a new segment).
#[tauri::command]
pub async fn resume_recording(state: tauri::State<'_, RecorderState>) -> Result<(), NarratorError> {
    let mut phase = state.phase.lock().await;
    if *phase != RecordingPhase::Paused {
        return Ok(());
    }
    *phase = RecordingPhase::Recording;
    drop(phase);

    let out_dir = state.output_dir.lock().await;
    let out_dir = out_dir
        .as_ref()
        .ok_or_else(|| NarratorError::FfmpegFailed("No output directory set".into()))?
        .clone();

    let seg_idx = state
        .segment_counter
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let (child, segment_path) = screen_recorder::start_segment(&out_dir, seg_idx, 30).await?;
    *state.ffmpeg_child.lock().await = Some(child);
    state.segments.lock().await.push(segment_path);

    tracing::info!("Recording resumed (segment {seg_idx})");
    Ok(())
}

/// Stop recording, concatenate segments, close overlay, emit result.
#[tauri::command]
pub async fn stop_screen_recording(
    app: tauri::AppHandle,
    state: tauri::State<'_, RecorderState>,
) -> Result<String, NarratorError> {
    let mut phase = state.phase.lock().await;
    if *phase == RecordingPhase::Idle || *phase == RecordingPhase::Stopping {
        return Err(NarratorError::FfmpegFailed(
            "No active recording to stop".to_string(),
        ));
    }
    let was_paused = *phase == RecordingPhase::Paused;
    *phase = RecordingPhase::Stopping;
    drop(phase);

    // Only stop current segment if not paused
    if !was_paused {
        let mut child_guard = state.ffmpeg_child.lock().await;
        if let Some(child) = child_guard.as_mut() {
            screen_recorder::stop_segment(child).await?;
        }
        *child_guard = None;
    }

    let segments = state.segments.lock().await.clone();
    let final_path = state
        .output_path
        .lock()
        .await
        .clone()
        .ok_or_else(|| NarratorError::FfmpegFailed("No output path set".into()))?;

    // Concatenate all segments
    let result_path = screen_recorder::concatenate_segments(&segments, &final_path).await?;

    // Close the overlay window
    if let Some(overlay) = app.get_webview_window("recorder") {
        let _ = overlay.close();
    }

    // Emit event for the main window
    use tauri::Emitter;
    let _ = app.emit("recording-stopped", &result_path);

    // Reset state
    state.reset().await;

    tracing::info!("Recording stopped → {result_path}");
    Ok(result_path)
}

/// Get the directory where recordings are saved.
#[tauri::command]
pub fn get_recordings_directory() -> Result<String, NarratorError> {
    let dir = screen_recorder::get_recordings_dir()?;
    Ok(dir.to_string_lossy().to_string())
}

// ── Video edit commands ──

/// Wrap a Tauri progress channel as a `ProgressReporter` so render functions
/// don't need to know about Tauri.
fn channel_reporter(channel: Channel<ProgressEvent>) -> Arc<dyn render::ProgressReporter> {
    Arc::new(render::FnReporter(move |event| {
        channel.send(event).ok();
    }))
}

#[tauri::command]
pub async fn apply_video_edits(
    input_path: String,
    output_path: String,
    edits: video_edit::VideoEditPlan,
    channel: Channel<ProgressEvent>,
) -> Result<String, NarratorError> {
    // Announce the phase BEFORE the first percent tick so the frontend can
    // light up the "Apply Edits" step card on the very first event. The
    // subsequent `ProgressEvent::Progress` stream (with optional messages)
    // flows through the shared reporter.
    channel
        .send(ProgressEvent::PhaseChange {
            phase: "applying_edits".to_string(),
        })
        .ok();
    render::apply_edits(&input_path, &output_path, &edits, channel_reporter(channel)).await
}

#[tauri::command]
pub async fn extract_edit_thumbnails(
    video_path: String,
    output_dir: String,
    count: usize,
) -> Result<Vec<String>, NarratorError> {
    render::extract_edit_thumbnails(&video_path, &output_dir, count).await
}

#[tauri::command]
pub async fn extract_single_frame(
    video_path: String,
    timestamp: f64,
    output_path: String,
) -> Result<String, NarratorError> {
    render::extract_single_frame(&video_path, timestamp, &output_path).await
}

#[tauri::command]
pub fn save_script(
    project_id: String,
    language: String,
    script: crate::models::NarrationScript,
) -> Result<String, NarratorError> {
    project_store::save_script(&project_id, &language, &script)
}

#[tauri::command]
pub async fn merge_audio_video(
    video_path: String,
    audio_path: String,
    output_path: String,
    replace_audio: bool,
    channel: Channel<ProgressEvent>,
    duck_db: Option<f32>,
) -> Result<render::MergeOutcome, NarratorError> {
    render::merge_audio_video(
        &video_path,
        &audio_path,
        &output_path,
        replace_audio,
        duck_db.unwrap_or(-8.0),
        channel_reporter(channel),
    )
    .await
}

#[tauri::command]
pub async fn open_folder(path: String) -> Result<(), NarratorError> {
    let p = Path::new(&path);
    if !p.is_dir() {
        return Err(NarratorError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Path is not a directory",
        )));
    }
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

    let frames = tokio::task::spawn_blocking(move || {
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

        let frames: Vec<ProjectFrame> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| ProjectFrame {
                index: i,
                path: entry.path().to_string_lossy().to_string(),
            })
            .collect();
        Ok::<_, NarratorError>(frames)
    })
    .await
    .map_err(|e| NarratorError::ProjectError(e.to_string()))??;
    Ok(frames)
}

// ── Export commands ──

#[tauri::command]
pub async fn export_script(options: ExportOptions) -> Result<Vec<ExportResult>, NarratorError> {
    let mut results = Vec::new();
    let output_dir = PathBuf::from(&options.output_directory);
    tokio::fs::create_dir_all(&output_dir)
        .await
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
                match tokio::fs::write(&filepath, &content).await {
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

/// Burn subtitles into `output_path`. When `audio_dir` is supplied and
/// contains a `narration_timings.json` sidecar (written by `generate_tts`
/// in compact mode), the SRT is generated from the **actual** TTS placements
/// rather than the scripted plan, keeping subs aligned with the audio even
/// when TTS overruns cause drift. If `cleanup_intermediate` is supplied,
/// that file is removed on success (typically the `_merged.mp4` the UI
/// produces on the way to the burnt final).
#[tauri::command]
pub async fn burn_subtitles(
    video_path: String,
    script: NarrationScript,
    output_path: String,
    channel: Channel<ProgressEvent>,
    style: Option<video_edit::SubtitleStyle>,
    audio_dir: Option<String>,
    cleanup_intermediate: Option<String>,
) -> Result<String, NarratorError> {
    let style = style.unwrap_or_default();

    // Try to load the sidecar timings written by generate_tts (compact mode).
    let actual_timings: Option<Vec<export_engine::ActualTiming>> = match audio_dir {
        Some(dir) => {
            let sidecar = std::path::PathBuf::from(&dir).join("narration_timings.json");
            match tokio::fs::read_to_string(&sidecar).await {
                Ok(s) => match serde_json::from_str::<Vec<export_engine::ActualTiming>>(&s) {
                    Ok(ts) => Some(ts),
                    Err(e) => {
                        tracing::warn!(
                            "narration_timings.json at {} was unparseable ({}), falling back to scripted timings",
                            sidecar.display(),
                            e
                        );
                        None
                    }
                },
                Err(_) => None,
            }
        }
        None => None,
    };

    let srt_content = export_engine::build_burn_srt(&script, actual_timings.as_deref());

    // Write SRT to a unique temp file. A shared path next to the output
    // used to race between concurrent exports and could survive a failed
    // burn as a stale file visible to the user.
    let srt_path =
        std::env::temp_dir().join(format!("_narrator_burn_srt_{}.srt", uuid::Uuid::new_v4()));
    tokio::fs::write(&srt_path, &srt_content).await?;

    let result = render::burn_subtitles(
        &video_path,
        &srt_path.to_string_lossy(),
        &output_path,
        &style,
        channel_reporter(channel),
    )
    .await;

    // Clean up the SRT no matter the outcome, and only remove the
    // caller-supplied intermediate if the burn actually succeeded — keep
    // the intermediate around on error so the user has something to inspect.
    let _ = tokio::fs::remove_file(&srt_path).await;
    if result.is_ok() {
        if let Some(path) = cleanup_intermediate {
            if is_safe_intermediate_path(&path, &output_path) {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::warn!(
                        "failed to remove intermediate {path}: {e} (burn succeeded, leaving file behind)"
                    );
                }
            } else {
                tracing::warn!(
                    "refusing to cleanup_intermediate {path:?}: must end with '_merged.mp4', live in the output directory, and differ from the output path"
                );
            }
        }
    }
    result
}

/// Guard the `cleanup_intermediate` IPC arg against deleting arbitrary
/// files. The UI passes `<output_dir>/<base>_merged.mp4`; anything else
/// (a different directory, a different filename, or the output path
/// itself) is rejected without even attempting the delete.
fn is_safe_intermediate_path(candidate: &str, output_path: &str) -> bool {
    if candidate.is_empty() || candidate == output_path {
        return false;
    }
    let cand = std::path::Path::new(candidate);
    let out = std::path::Path::new(output_path);
    let Some(cand_name) = cand.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    // We only ever produce `<base>_merged.mp4` as an intermediate. Insist
    // on that suffix so a future UI bug can't ask us to delete something
    // else.
    if !cand_name.ends_with("_merged.mp4") {
        return false;
    }
    match (cand.parent(), out.parent()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

// ── Style commands ──

#[tauri::command]
pub async fn list_styles() -> Result<Vec<NarrationStyle>, NarratorError> {
    project_store::load_styles()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_safe_intermediate_path tests ──

    #[test]
    fn is_safe_intermediate_accepts_sibling_merged_mp4() {
        assert!(is_safe_intermediate_path(
            "/tmp/out/video_merged.mp4",
            "/tmp/out/video_burned.mp4"
        ));
    }

    #[test]
    fn is_safe_intermediate_rejects_output_path() {
        let out = "/tmp/out/video.mp4";
        assert!(!is_safe_intermediate_path(out, out));
    }

    #[test]
    fn is_safe_intermediate_rejects_non_merged_suffix() {
        assert!(!is_safe_intermediate_path(
            "/tmp/out/important.mov",
            "/tmp/out/video.mp4"
        ));
    }

    #[test]
    fn is_safe_intermediate_rejects_different_directory() {
        assert!(!is_safe_intermediate_path(
            "/var/other/video_merged.mp4",
            "/tmp/out/video.mp4"
        ));
    }

    #[test]
    fn is_safe_intermediate_rejects_empty_and_bare_filename() {
        assert!(!is_safe_intermediate_path("", "/tmp/out/video.mp4"));
        // A bare filename has no parent — reject rather than risk resolving
        // against the process cwd.
        assert!(!is_safe_intermediate_path(
            "video_merged.mp4",
            "/tmp/out/video.mp4"
        ));
    }

    // ── sanitize_tts_text tests ──

    #[test]
    fn test_sanitize_tts_text_removes_pause_markers() {
        assert_eq!(sanitize_tts_text("Hello [pause] world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello [break] world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello (pause) world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello (break) world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello [silence] world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello [Pause] world"), "Hello world");
        assert_eq!(sanitize_tts_text("Hello [PAUSE] world"), "Hello world");
    }

    #[test]
    fn test_sanitize_tts_text_removes_multiple_markers() {
        assert_eq!(
            sanitize_tts_text("Start [pause] middle [break] end"),
            "Start middle end"
        );
    }

    #[test]
    fn test_sanitize_tts_text_preserves_normal() {
        assert_eq!(sanitize_tts_text("Hello world"), "Hello world");
        assert_eq!(
            sanitize_tts_text("This is a normal sentence."),
            "This is a normal sentence."
        );
        assert_eq!(
            sanitize_tts_text("Numbers 123 and symbols!"),
            "Numbers 123 and symbols!"
        );
    }

    #[test]
    fn test_sanitize_tts_text_whitespace() {
        // Multiple spaces are collapsed to single spaces
        assert_eq!(sanitize_tts_text("Hello   world"), "Hello world");
        assert_eq!(
            sanitize_tts_text("  leading and trailing  "),
            "leading and trailing"
        );
        assert_eq!(sanitize_tts_text("Hello  [pause]  world"), "Hello world");
    }

    #[test]
    fn test_sanitize_tts_text_empty_string() {
        assert_eq!(sanitize_tts_text(""), "");
    }

    // ── tts_cache_key tests ──

    fn make_elevenlabs_provider() -> TtsProvider {
        TtsProvider::ElevenLabs(elevenlabs_client::ElevenLabsConfig {
            api_key: "test-key".to_string(),
            voice_id: "JBFqnCBsd6RMkjVDRZzb".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.0,
        })
    }

    fn make_azure_provider() -> TtsProvider {
        TtsProvider::Azure(azure_tts_client::AzureTtsConfig {
            api_key: "test-key".to_string(),
            region: "eastus".to_string(),
            voice_name: "en-US-JennyNeural".to_string(),
            speaking_style: "chat".to_string(),
            speed: 1.0,
        })
    }

    #[test]
    fn test_tts_cache_key_deterministic() {
        let provider = make_elevenlabs_provider();
        let key1 = tts_cache_key(&provider, "Hello world");
        let key2 = tts_cache_key(&provider, "Hello world");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_tts_cache_key_different_text() {
        let provider = make_elevenlabs_provider();
        let key1 = tts_cache_key(&provider, "Hello world");
        let key2 = tts_cache_key(&provider, "Goodbye world");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_tts_cache_key_different_provider_settings() {
        let el_provider = make_elevenlabs_provider();
        let az_provider = make_azure_provider();
        let key1 = tts_cache_key(&el_provider, "Hello world");
        let key2 = tts_cache_key(&az_provider, "Hello world");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_tts_cache_key_different_voice() {
        let provider1 = make_elevenlabs_provider();
        let provider2 = TtsProvider::ElevenLabs(elevenlabs_client::ElevenLabsConfig {
            api_key: "test-key".to_string(),
            voice_id: "different-voice-id".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.0,
        });
        let key1 = tts_cache_key(&provider1, "Hello world");
        let key2 = tts_cache_key(&provider2, "Hello world");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_tts_cache_key_different_speed() {
        let provider1 = make_elevenlabs_provider();
        let provider2 = TtsProvider::ElevenLabs(elevenlabs_client::ElevenLabsConfig {
            api_key: "test-key".to_string(),
            voice_id: "JBFqnCBsd6RMkjVDRZzb".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.5,
        });
        let key1 = tts_cache_key(&provider1, "Hello world");
        let key2 = tts_cache_key(&provider2, "Hello world");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_tts_cache_key_api_key_not_in_hash() {
        // Different API keys should produce the same cache key (API key is not used in hash)
        let provider1 = make_elevenlabs_provider();
        let provider2 = TtsProvider::ElevenLabs(elevenlabs_client::ElevenLabsConfig {
            api_key: "different-api-key".to_string(),
            voice_id: "JBFqnCBsd6RMkjVDRZzb".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.0,
        });
        let key1 = tts_cache_key(&provider1, "Hello world");
        let key2 = tts_cache_key(&provider2, "Hello world");
        assert_eq!(key1, key2);
    }

    // ── Builtin TTS cache key tests ──

    #[test]
    fn test_tts_cache_key_builtin() {
        let provider = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        let key = tts_cache_key(&provider, "Hello world");
        // Should produce a valid non-empty hex string
        assert!(!key.is_empty());
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_tts_cache_key_builtin_deterministic() {
        let provider1 = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        let provider2 = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        assert_eq!(
            tts_cache_key(&provider1, "Hello"),
            tts_cache_key(&provider2, "Hello")
        );
    }

    #[test]
    fn test_tts_cache_key_builtin_different_voice() {
        let provider1 = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        let provider2 = TtsProvider::Builtin {
            voice: "Samantha".to_string(),
            speed: 1.0,
        };
        assert_ne!(
            tts_cache_key(&provider1, "Hello"),
            tts_cache_key(&provider2, "Hello")
        );
    }

    #[test]
    fn test_tts_cache_key_builtin_different_speed() {
        let provider1 = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        let provider2 = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.5,
        };
        assert_ne!(
            tts_cache_key(&provider1, "Hello"),
            tts_cache_key(&provider2, "Hello")
        );
    }

    #[test]
    fn test_tts_cache_key_builtin_differs_from_other_providers() {
        let builtin = TtsProvider::Builtin {
            voice: "default".to_string(),
            speed: 1.0,
        };
        let el = make_elevenlabs_provider();
        let az = make_azure_provider();
        let text = "Hello world";
        let key_builtin = tts_cache_key(&builtin, text);
        let key_el = tts_cache_key(&el, text);
        let key_az = tts_cache_key(&az, text);
        assert_ne!(key_builtin, key_el);
        assert_ne!(key_builtin, key_az);
    }
}
