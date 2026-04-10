//! Tauri command handlers for the Narrator application.

use crate::error::NarratorError;
use crate::models::*;
use crate::process_utils::CommandNoWindow;
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
    pub ffmpeg_child: Arc<Mutex<Option<tokio::process::Child>>>,
    pub segments: Arc<Mutex<Vec<String>>>,
    pub phase: Arc<Mutex<RecordingPhase>>,
    pub output_dir: Arc<Mutex<Option<String>>>,
    pub output_path: Arc<Mutex<Option<String>>>,
    pub segment_counter: Arc<std::sync::atomic::AtomicU32>,
}

impl RecorderState {
    pub fn new() -> Self {
        Self {
            ffmpeg_child: Arc::new(Mutex::new(None)),
            segments: Arc::new(Mutex::new(Vec::new())),
            phase: Arc::new(Mutex::new(RecordingPhase::Idle)),
            output_dir: Arc::new(Mutex::new(None)),
            output_path: Arc::new(Mutex::new(None)),
            segment_counter: Arc::new(std::sync::atomic::AtomicU32::new(0)),
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
    std::fs::create_dir_all(&frames_dir)
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let channel_clone = channel.clone();
    let frames_dir_cleanup = frames_dir.clone();
    let frames = match video_engine::extract_frames(
        Path::new(&params.video_path),
        &frame_config,
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

    // Start estimated progress reporter — gives the user a smooth progress bar
    // during the 30-60 second AI call instead of a frozen UI.
    let progress_channel = channel.clone();
    let cancel_progress = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_progress.clone();

    let progress_task = tokio::spawn(async move {
        let mut pct = 0.0_f64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if cancel_clone.load(Ordering::SeqCst) {
                break;
            }
            // Logarithmic progress: approaches but never reaches 95%
            pct += (95.0 - pct) * 0.15;
            progress_channel
                .send(ProgressEvent::Progress { percent: pct })
                .ok();
        }
    });

    let script = ai_client::generate_narration(
        provider.as_ref(),
        &system_prompt,
        user_message,
        &params.style,
        &params.primary_language,
    )
    .await;

    // Stop estimated progress reporter
    cancel_progress.store(true, Ordering::SeqCst);
    let _ = progress_task.await;

    // Jump to 100% once the AI call completes
    channel
        .send(ProgressEvent::Progress { percent: 100.0 })
        .ok();

    let script = script?;

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
        edit_clips: None,
        video_metadata: None,
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
    project_store::validate_project_id(&id)?;
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
            match std::fs::copy(&cached, filepath) {
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

    // Store in cache
    if let Ok(cache_dir) = tts_cache_dir() {
        let cached = cache_dir.join(format!("{cache_key}.mp3"));
        let _ = std::fs::copy(filepath, &cached);
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

            match generate_speech_for_provider(&tts, &seg.text, &filepath).await {
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

            // Build ordered list of files: silence gaps interleaved with segments.
            // Track the running audio position to compute gaps accurately,
            // since TTS audio duration often differs from the segment time window.
            let mut concat_parts: Vec<PathBuf> = Vec::new();
            let mut silence_idx = 0;
            let mut audio_pos: f64 = 0.0; // current position in the output audio

            for (_seg_idx, seg_path, start, _end) in segment_files.iter() {
                // Gap = desired start position minus where we currently are
                let gap = start - audio_pos;

                if gap > 0.05 {
                    let sil_path = out.join(format!("_tmp_sil_{}.mp3", silence_idx));
                    let anullsrc = format!("anullsrc=r={}:cl=stereo", silence_sample_rate);
                    let _ = tokio::process::Command::new(ffmpeg.as_os_str())
                        .no_window()
                        .args(["-y", "-f", "lavfi", "-i"])
                        .arg(&anullsrc)
                        .args([
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
                    audio_pos += gap;
                    silence_idx += 1;
                }

                // Probe actual TTS audio duration
                let seg_dur = video_engine::probe_duration(seg_path.as_path())
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!("Could not probe TTS segment duration: {e}, estimating from time window");
                        (_end - start).max(0.5)
                    });

                concat_parts.push(seg_path.clone());
                audio_pos += seg_dur;
            }

            // Trailing silence to reach full video duration
            let last_end = segment_files.last().map(|s| s.3).unwrap_or(0.0);
            if video_dur > last_end + 0.1 {
                let trail = video_dur - last_end;
                let sil_path = out.join(format!("_tmp_sil_{}.mp3", silence_idx));
                let anullsrc = format!("anullsrc=r={}:cl=stereo", silence_sample_rate);
                let _ = tokio::process::Command::new(ffmpeg.as_os_str())
                    .no_window()
                    .args(["-y", "-f", "lavfi", "-i"])
                    .arg(&anullsrc)
                    .args([
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
                .map(|p| {
                    let escaped = p
                        .to_string_lossy()
                        .replace('\\', "\\\\")
                        .replace(['\n', '\r'], "")
                        .replace('\'', "'\\''");
                    format!("file '{}'", escaped)
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&concat_list_path, &concat_content)?;

            channel.send(ProgressEvent::Progress { percent: 92.0 }).ok();

            // Run concat
            let concat_output = tokio::process::Command::new(ffmpeg.as_os_str())
                .no_window()
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

            match generate_speech_for_provider(&tts, &seg.text, &filepath).await {
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
    channel: Channel<ProgressEvent>,
) -> Result<String, NarratorError> {
    video_edit::merge_audio_video(
        &video_path,
        &audio_path,
        &output_path,
        replace_audio,
        |pct| {
            channel.send(ProgressEvent::Progress { percent: pct }).ok();
        },
    )
    .await
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
    channel: Channel<ProgressEvent>,
) -> Result<String, NarratorError> {
    // Write SRT to temp file, then burn
    let out_dir = std::path::Path::new(&output_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let srt_path = out_dir.join("_temp_subtitles.srt");
    std::fs::write(&srt_path, &srt_content)?;

    let result = video_edit::burn_subtitles(
        &video_path,
        &srt_path.to_string_lossy(),
        &output_path,
        |pct| {
            channel.send(ProgressEvent::Progress { percent: pct }).ok();
        },
    )
    .await;

    let _ = std::fs::remove_file(&srt_path);
    result
}

// ── Style commands ──

#[tauri::command]
pub async fn list_styles() -> Result<Vec<NarrationStyle>, NarratorError> {
    project_store::load_styles()
}

#[cfg(test)]
mod tests {
    use super::*;

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
