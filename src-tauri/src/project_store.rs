//! Project persistence and management on the local filesystem.

use crate::error::NarratorError;
use crate::models::*;
use std::path::{Path, PathBuf};

/// Validate that a project ID is a valid UUID to prevent path traversal attacks.
pub fn validate_project_id(id: &str) -> Result<(), NarratorError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| NarratorError::ProjectError(format!("Invalid project ID: {id}")))?;
    Ok(())
}

/// Atomically write `contents` to `path` by writing to a sibling temp file and
/// renaming it over the target. If the process is killed mid-write, the target
/// is either the old version or the new version — never a truncated file.
///
/// This is critical for `project.json` and script files: a crash during a
/// non-atomic write leaves the user with empty or corrupt JSON and loses edits.
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    // Build a unique sibling temp path so concurrent writes don't collide.
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp");
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp = parent.join(format!(".{file_name}.tmp.{nonce}"));

    // Write + sync + rename. On POSIX, rename over an existing file is atomic.
    // On Windows, it's atomic since Windows 10 build 10586 for NTFS.
    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();

    if write_result.is_err() {
        // Best-effort cleanup of the temp file
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

pub fn get_narrator_dir() -> PathBuf {
    // Test/dev override: NARRATOR_DIR lets integration tests and scripts
    // point the store at a sandboxed directory instead of ~/.narrator.
    if let Ok(override_dir) = std::env::var("NARRATOR_DIR") {
        if !override_dir.is_empty() {
            return PathBuf::from(override_dir);
        }
    }
    if let Some(home) = directories::UserDirs::new() {
        home.home_dir().join(".narrator")
    } else if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        PathBuf::from(home).join(".narrator")
    } else {
        // Last resort: use system temp dir (not /tmp directly)
        std::env::temp_dir().join(".narrator")
    }
}

pub fn ensure_directories() -> Result<(), NarratorError> {
    let base = get_narrator_dir();
    let dirs = [
        base.clone(),
        base.join("projects"),
        base.join("styles"),
        base.join("context_library"),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir).map_err(|e| {
            NarratorError::ProjectError(format!("Failed to create {}: {e}", dir.display()))
        })?;
    }
    Ok(())
}

pub fn create_project(config: &ProjectConfig) -> Result<String, NarratorError> {
    validate_project_id(&config.id)?;
    let base = get_narrator_dir();
    let project_dir = base.join("projects").join(&config.id);

    std::fs::create_dir_all(&project_dir)
        .map_err(|e| NarratorError::ProjectError(format!("Failed to create project dir: {e}")))?;
    std::fs::create_dir_all(project_dir.join("frames"))
        .map_err(|e| NarratorError::ProjectError(format!("Failed to create frames dir: {e}")))?;
    std::fs::create_dir_all(project_dir.join("scripts"))
        .map_err(|e| NarratorError::ProjectError(format!("Failed to create scripts dir: {e}")))?;
    std::fs::create_dir_all(project_dir.join("exports"))
        .map_err(|e| NarratorError::ProjectError(format!("Failed to create exports dir: {e}")))?;

    let json = serde_json::to_string_pretty(config)?;
    atomic_write(&project_dir.join("project.json"), json.as_bytes())
        .map_err(|e| NarratorError::ProjectError(format!("Failed to write project.json: {e}")))?;

    Ok(config.id.clone())
}

#[allow(dead_code)]
pub fn save_project(config: &ProjectConfig) -> Result<(), NarratorError> {
    let base = get_narrator_dir();
    let project_dir = base.join("projects").join(&config.id);

    if !project_dir.exists() {
        create_project(config)?;
        return Ok(());
    }

    let json = serde_json::to_string_pretty(config)?;
    atomic_write(&project_dir.join("project.json"), json.as_bytes())
        .map_err(|e| NarratorError::ProjectError(format!("Failed to save project: {e}")))?;

    Ok(())
}

pub fn load_project(id: &str) -> Result<ProjectConfig, NarratorError> {
    validate_project_id(id)?;
    let base = get_narrator_dir();
    let project_file = base.join("projects").join(id).join("project.json");

    if !project_file.exists() {
        return Err(NarratorError::ProjectError(format!(
            "Project not found: {id}"
        )));
    }

    let json = std::fs::read_to_string(&project_file)
        .map_err(|e| NarratorError::ProjectError(format!("Failed to read project: {e}")))?;

    serde_json::from_str(&json)
        .map_err(|e| NarratorError::ProjectError(format!("Failed to parse project: {e}")))
}

pub fn list_projects() -> Result<Vec<ProjectSummary>, NarratorError> {
    let base = get_narrator_dir();
    let projects_dir = base.join("projects");

    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();

    for entry in std::fs::read_dir(&projects_dir)
        .map_err(|e| NarratorError::ProjectError(format!("Failed to read projects dir: {e}")))?
    {
        let entry = entry.map_err(|e| NarratorError::ProjectError(e.to_string()))?;
        let project_file = entry.path().join("project.json");

        if project_file.exists() {
            if let Ok(json) = std::fs::read_to_string(&project_file) {
                if let Ok(config) = serde_json::from_str::<ProjectConfig>(&json) {
                    let scripts_dir = entry.path().join("scripts");
                    let script_languages = list_script_languages(&scripts_dir);
                    let has_script = !script_languages.is_empty();

                    // Find thumbnail: first frame in frames dir
                    let frames_dir = entry.path().join("frames");
                    let thumbnail_path = find_thumbnail(&frames_dir);

                    summaries.push(ProjectSummary {
                        id: config.id,
                        title: config.title,
                        video_path: config.video_path,
                        style: config.style,
                        created_at: config.created_at,
                        updated_at: config.updated_at,
                        has_script,
                        thumbnail_path,
                        script_languages,
                    });
                }
            }
        }
    }

    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(summaries)
}

fn find_thumbnail(frames_dir: &Path) -> Option<String> {
    if !frames_dir.exists() {
        return None;
    }
    let mut entries: Vec<_> = std::fs::read_dir(frames_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "jpg" || x == "jpeg" || x == "png")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
    entries
        .first()
        .map(|e| e.path().to_string_lossy().to_string())
}

fn list_script_languages(scripts_dir: &Path) -> Vec<String> {
    if !scripts_dir.exists() {
        return Vec::new();
    }
    let mut langs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(scripts_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // Files like v1_en.json → extract "en"
            if let Some(stem) = name.strip_suffix(".json") {
                if let Some(lang) = stem.split('_').next_back() {
                    if !langs.contains(&lang.to_string()) {
                        langs.push(lang.to_string());
                    }
                }
            }
        }
    }
    langs
}

pub fn load_project_full(id: &str) -> Result<LoadedProject, NarratorError> {
    validate_project_id(id)?;
    let config = load_project(id)?;

    let base = get_narrator_dir();
    let scripts_dir = base.join("projects").join(id).join("scripts");
    let mut scripts = std::collections::HashMap::new();

    if scripts_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            // First pass: find the latest filename per language (avoids reading old versions)
            let mut latest_per_lang: std::collections::HashMap<String, (String, PathBuf)> =
                std::collections::HashMap::new();

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|x| x == "json") {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Some(stem) = name.strip_suffix(".json") {
                        if let Some(lang) = stem.split('_').next_back() {
                            let lang = lang.to_string();
                            let should_replace = match latest_per_lang.get(&lang) {
                                Some((existing_name, _)) => name > *existing_name,
                                None => true,
                            };
                            if should_replace {
                                latest_per_lang.insert(lang, (name, path));
                            }
                        }
                    }
                }
            }

            // Second pass: only read the latest file per language
            for (lang, (_name, path)) in latest_per_lang {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    if let Ok(script) = serde_json::from_str::<NarrationScript>(&json) {
                        scripts.insert(lang, script);
                    }
                }
            }
        }
    }

    Ok(LoadedProject { config, scripts })
}

pub fn save_script(
    project_id: &str,
    language: &str,
    script: &NarrationScript,
) -> Result<String, NarratorError> {
    validate_project_id(project_id)?;
    let base = get_narrator_dir();
    let scripts_dir = base.join("projects").join(project_id).join("scripts");
    std::fs::create_dir_all(&scripts_dir)
        .map_err(|e| NarratorError::ProjectError(e.to_string()))?;

    // Find next version number
    let version = find_next_version(&scripts_dir, language);
    let filename = format!("v{}_{}.json", version, language);
    let filepath = scripts_dir.join(&filename);

    let json = serde_json::to_string_pretty(script)?;
    atomic_write(&filepath, json.as_bytes())
        .map_err(|e| NarratorError::ProjectError(format!("Failed to save script: {e}")))?;

    Ok(filepath.to_string_lossy().to_string())
}

fn find_next_version(scripts_dir: &Path, language: &str) -> u32 {
    let mut max_version = 0u32;

    if let Ok(entries) = std::fs::read_dir(scripts_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains(language) && name.starts_with('v') {
                if let Some(version_str) = name.strip_prefix('v').and_then(|s| s.split('_').next())
                {
                    if let Ok(v) = version_str.parse::<u32>() {
                        max_version = max_version.max(v);
                    }
                }
            }
        }
    }

    max_version + 1
}

pub fn get_project_frames_dir(project_id: &str) -> PathBuf {
    get_narrator_dir()
        .join("projects")
        .join(project_id)
        .join("frames")
}

// ── Project templates ────────────────────────────────────────────────────────

pub fn save_template(template: &ProjectTemplate) -> Result<(), NarratorError> {
    let dir = get_narrator_dir().join("templates");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", template.id));
    let json = serde_json::to_string_pretty(template)?;
    atomic_write(&path, json.as_bytes())?;
    Ok(())
}

pub fn list_templates() -> Result<Vec<ProjectTemplate>, NarratorError> {
    let dir = get_narrator_dir().join("templates");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut templates = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(json) = std::fs::read_to_string(&path) {
                if let Ok(t) = serde_json::from_str::<ProjectTemplate>(&json) {
                    templates.push(t);
                }
            }
        }
    }
    templates.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(templates)
}

pub fn delete_template(id: &str) -> Result<(), NarratorError> {
    let path = get_narrator_dir()
        .join("templates")
        .join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

// ── Project export/import ───────────────────────────────────────────────────

/// Export a project to a portable `.narrator` file (ZIP archive).
/// Includes project.json, all scripts, and extracted frames.
/// Video paths are stored as relative references (not bundled — too large).
pub fn export_project(project_id: &str, output_path: &Path) -> Result<(), NarratorError> {
    validate_project_id(project_id)?;
    let base = get_narrator_dir();
    let project_dir = base.join("projects").join(project_id);

    if !project_dir.exists() {
        return Err(NarratorError::ProjectError(format!(
            "Project not found: {project_id}"
        )));
    }

    let file = std::fs::File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Add project.json
    let config_path = project_dir.join("project.json");
    if config_path.exists() {
        zip.start_file("project.json", options)
            .map_err(|e| NarratorError::ProjectError(format!("ZIP error: {e}")))?;
        let data = std::fs::read(&config_path)?;
        std::io::Write::write_all(&mut zip, &data)?;
    }

    // Add scripts
    let scripts_dir = project_dir.join("scripts");
    if scripts_dir.exists() {
        for entry in std::fs::read_dir(&scripts_dir)?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let name = format!("scripts/{}", entry.file_name().to_string_lossy());
                zip.start_file(&name, options)
                    .map_err(|e| NarratorError::ProjectError(format!("ZIP error: {e}")))?;
                let data = std::fs::read(&path)?;
                std::io::Write::write_all(&mut zip, &data)?;
            }
        }
    }

    // Add frames
    let frames_dir = project_dir.join("frames");
    if frames_dir.exists() {
        for entry in std::fs::read_dir(&frames_dir)?.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e == "jpg" || e == "jpeg" || e == "png")
            {
                let name = format!("frames/{}", entry.file_name().to_string_lossy());
                zip.start_file(&name, options)
                    .map_err(|e| NarratorError::ProjectError(format!("ZIP error: {e}")))?;
                let data = std::fs::read(&path)?;
                std::io::Write::write_all(&mut zip, &data)?;
            }
        }
    }

    zip.finish()
        .map_err(|e| NarratorError::ProjectError(format!("Failed to finalize ZIP: {e}")))?;

    Ok(())
}

/// Import a `.narrator` file (ZIP archive) as a new project.
/// Assigns a new UUID to avoid ID collisions. Video path is cleared
/// (user must re-link the video on the new machine).
pub fn import_project(archive_path: &Path) -> Result<String, NarratorError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| NarratorError::ProjectError(format!("Invalid .narrator file: {e}")))?;

    // Read project.json to get metadata
    let mut config: ProjectConfig = {
        let mut entry = archive.by_name("project.json").map_err(|e| {
            NarratorError::ProjectError(format!("Missing project.json in archive: {e}"))
        })?;
        let mut json = String::new();
        std::io::Read::read_to_string(&mut entry, &mut json)?;
        serde_json::from_str(&json)
            .map_err(|e| NarratorError::ProjectError(format!("Invalid project.json: {e}")))?
    };

    // Assign new UUID to avoid collisions
    let new_id = uuid::Uuid::new_v4().to_string();
    config.id = new_id.clone();

    // Mark video path as needing re-link (not portable across machines)
    config.video_path = String::new();
    config.video_metadata = None;

    // Update timestamps
    let now = chrono::Utc::now().to_rfc3339();
    config.updated_at = now.clone();
    // Append "(imported)" to title so user knows it came from elsewhere
    if !config.title.ends_with("(imported)") {
        config.title = format!("{} (imported)", config.title);
    }

    // Create project directory structure
    let base = get_narrator_dir();
    let project_dir = base.join("projects").join(&new_id);
    std::fs::create_dir_all(project_dir.join("frames"))?;
    std::fs::create_dir_all(project_dir.join("scripts"))?;
    std::fs::create_dir_all(project_dir.join("exports"))?;

    // Write updated project.json
    let json = serde_json::to_string_pretty(&config)?;
    atomic_write(&project_dir.join("project.json"), json.as_bytes())?;

    // Extract scripts and frames
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| NarratorError::ProjectError(format!("ZIP entry error: {e}")))?;

        let name = entry.name().to_string();

        // Skip project.json (already handled) and any directory entries
        if name == "project.json" || entry.is_dir() {
            continue;
        }

        // Only extract known paths (security: prevent path traversal)
        let dest = if let Some(script_name) = name.strip_prefix("scripts/") {
            if script_name.contains('/') || script_name.contains('\\') {
                continue;
            }
            project_dir.join("scripts").join(script_name)
        } else if let Some(frame_name) = name.strip_prefix("frames/") {
            if frame_name.contains('/') || frame_name.contains('\\') {
                continue;
            }
            project_dir.join("frames").join(frame_name)
        } else {
            continue; // Skip unknown entries
        };

        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data)?;
        std::fs::write(&dest, &data)?;
    }

    Ok(new_id)
}

pub fn load_styles() -> Result<Vec<NarrationStyle>, NarratorError> {
    let styles_dir = get_narrator_dir().join("styles");

    if !styles_dir.exists()
        || std::fs::read_dir(&styles_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    {
        return Ok(default_styles());
    }

    let mut styles = Vec::new();

    for entry in std::fs::read_dir(&styles_dir)
        .map_err(|e| NarratorError::ProjectError(format!("Failed to read styles dir: {e}")))?
    {
        let entry = entry.map_err(|e| NarratorError::ProjectError(e.to_string()))?;
        if entry.path().extension().is_some_and(|e| e == "toml") {
            let content = std::fs::read_to_string(entry.path())
                .map_err(|e| NarratorError::ProjectError(e.to_string()))?;
            if let Ok(style) = toml::from_str::<NarrationStyle>(&content) {
                styles.push(style);
            }
        }
    }

    if styles.is_empty() {
        return Ok(default_styles());
    }

    Ok(styles)
}

pub fn default_styles() -> Vec<NarrationStyle> {
    vec![
        NarrationStyle {
            id: "executive".to_string(),
            label: "Executive Overview".to_string(),
            description: "Confident, outcome-focused, minimal jargon. Emphasizes business value, ROI, and strategic impact.".to_string(),
            system_prompt: "You are a senior business narrator creating an executive-level overview.\n\n\
                TONE & VOICE:\n\
                - Confident, authoritative, and outcome-focused\n\
                - Minimal technical jargon — translate features into business outcomes\n\
                - Short, decisive sentences that respect the viewer's time\n\
                - Speak to C-suite, VP, and director-level audiences\n\n\
                CONTENT STRATEGY:\n\
                - Lead with the \"so what\" — why this matters to the business\n\
                - Frame every feature as a business capability: cost savings, efficiency gains, risk reduction\n\
                - Use metrics-oriented language: \"reduces time by\", \"eliminates manual steps\", \"scales across\"\n\
                - End segments with forward-looking statements about impact\n\n\
                WHAT TO AVOID:\n\
                - Implementation details, code, or technical configuration\n\
                - Hedging language (\"maybe\", \"potentially\", \"sort of\")\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "medium".to_string(),
            pause_markers: false,
        },
        NarrationStyle {
            id: "product_demo".to_string(),
            label: "Product Demo".to_string(),
            description: "Polished and enthusiastic walkthrough for customers and partners.".to_string(),
            system_prompt: "You are a product specialist narrating a polished demo for customers and partners.\n\n\
                TONE & VOICE:\n\
                - Enthusiastic but not salesy — genuinely excited about solving real problems\n\
                - Conversational and accessible, like a knowledgeable colleague\n\
                - Second-person framing: \"you can\", \"this lets you\", \"here's where you would\"\n\n\
                CONTENT STRATEGY:\n\
                - Walk through each visible feature step by step as it appears on screen\n\
                - Explain WHAT the user is seeing, WHAT it does, and WHY it matters\n\
                - Connect each feature to a real workflow or pain point it solves\n\
                - Highlight ease of use, time savings, and \"aha\" moments\n\
                - Build excitement progressively\n\n\
                WHAT TO AVOID:\n\
                - Raw technical specs without user benefit\n\
                - Competitive comparisons or negative framing\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "medium".to_string(),
            pause_markers: false,
        },
        NarrationStyle {
            id: "technical".to_string(),
            label: "Technical Deep-Dive".to_string(),
            description: "Precise, developer-oriented. Names UI elements, APIs, config options explicitly.".to_string(),
            system_prompt: "You are a senior engineer narrating a technical deep-dive for a developer audience.\n\n\
                TONE & VOICE:\n\
                - Precise, confident, and developer-oriented\n\
                - Direct and efficient — no filler, every word earns its place\n\
                - Imperative voice: \"Open the settings\", \"Configure the endpoint\"\n\
                - Assume strong domain knowledge\n\n\
                CONTENT STRATEGY:\n\
                - Name every UI element, button, menu, panel, and field visible on screen\n\
                - Reference APIs, config keys, environment variables explicitly\n\
                - Explain the WHY behind each configuration choice\n\
                - Call out architectural decisions, patterns, and trade-offs\n\
                - Note prerequisites, dependencies, or setup steps implied by what's shown\n\n\
                WHAT TO AVOID:\n\
                - Vague descriptions (\"click the thing\", \"change the setting\")\n\
                - Marketing language or business justification\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "medium".to_string(),
            pause_markers: false,
        },
        NarrationStyle {
            id: "teaser".to_string(),
            label: "Teaser / Trailer".to_string(),
            description: "High-energy, short punchy sentences. Focus on wow moments.".to_string(),
            system_prompt: "You are creating a high-energy teaser or trailer narration that builds excitement.\n\n\
                TONE & VOICE:\n\
                - Dynamic, bold, and forward-looking\n\
                - Short punchy sentences with strong verbs\n\
                - Build momentum — each segment should escalate the energy\n\
                - Speak like a movie trailer narrator: confident, dramatic, compelling\n\n\
                CONTENT STRATEGY:\n\
                - Focus on the most impressive visual moments on screen\n\
                - Create a narrative arc: hook, build, peak, call to action\n\
                - Use power words: transform, revolutionize, unleash, seamless, instant\n\
                - Frame capabilities as transformative outcomes, not features\n\
                - End with a strong call to action or forward-looking vision\n\n\
                WHAT TO AVOID:\n\
                - Long explanations or step-by-step walkthroughs\n\
                - Technical details or configuration specifics\n\
                - Passive or cautious language\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "fast".to_string(),
            pause_markers: false,
        },
        NarrationStyle {
            id: "training".to_string(),
            label: "Training Walkthrough".to_string(),
            description: "Patient, methodical, instructional. Includes callouts for common mistakes.".to_string(),
            system_prompt: "You are an experienced instructor narrating a training walkthrough.\n\n\
                TONE & VOICE:\n\
                - Patient, methodical, and encouraging\n\
                - Speak as if the viewer is following along on their own screen\n\
                - Use clear, simple language — avoid jargon unless you define it\n\n\
                CONTENT STRATEGY:\n\
                - Describe every action visible on screen in sequential order\n\
                - Use numbered steps when walking through a multi-step process\n\
                - Explain not just WHAT to click, but WHERE to find it and WHY\n\
                - Call out common mistakes, gotchas, and \"if you see this, do that\" scenarios\n\
                - Include verification points: \"You should now see...\"\n\
                - Pace the narration so the viewer can follow without pausing\n\n\
                WHAT TO AVOID:\n\
                - Rushing through steps or skipping explanations\n\
                - Assuming the viewer knows where things are located\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "slow".to_string(),
            pause_markers: false,
        },
        NarrationStyle {
            id: "critique".to_string(),
            label: "Bug Review / Critique".to_string(),
            description: "Analytical review identifying issues, UX problems, and improvements.".to_string(),
            system_prompt: "You are a senior QA analyst and UX reviewer narrating a critical assessment.\n\n\
                TONE & VOICE:\n\
                - Analytical, constructive, and detail-oriented\n\
                - Objective and evidence-based — describe what you see, then assess it\n\
                - Professional but direct — don't sugarcoat issues\n\
                - Balance criticism with acknowledgment of what works well\n\n\
                CONTENT STRATEGY:\n\
                - Describe exactly what's visible on screen before making any assessment\n\
                - Identify potential bugs: visual glitches, misalignment, unexpected states\n\
                - Evaluate UX quality: is the flow intuitive? Are labels clear?\n\
                - Note accessibility concerns: contrast, font sizes, clickable areas\n\
                - Suggest specific improvements with reasoning\n\
                - Prioritize: critical issues before minor polish items\n\n\
                WHAT TO AVOID:\n\
                - Vague complaints without specific references\n\
                - Speculation about code not visible on screen\n\
                - Only negative feedback — acknowledge good design too\n\
                - Any markup, tags, or non-speakable text like [pause] or (break)".to_string(),
            pacing: "medium".to_string(),
            pause_markers: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_narrator_dir() {
        // Must acquire the store lock so no other test mutates NARRATOR_DIR
        // while we probe it. Explicitly unset it first so the assertion
        // against ".narrator" path segment holds.
        let _lock = STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: lock held, no other test is touching env right now.
        unsafe {
            std::env::remove_var("NARRATOR_DIR");
        }
        let dir = get_narrator_dir();
        assert!(dir.to_string_lossy().contains(".narrator"));
    }

    #[test]
    fn test_create_and_load_project() {
        let temp = tempfile::tempdir().unwrap();
        // Override the narrator dir for testing
        let project_id = uuid::Uuid::new_v4().to_string();
        let project_dir = temp.path().join(&project_id);
        std::fs::create_dir_all(project_dir.join("frames")).unwrap();
        std::fs::create_dir_all(project_dir.join("scripts")).unwrap();
        std::fs::create_dir_all(project_dir.join("exports")).unwrap();

        let config = ProjectConfig {
            schema_version: 1,
            id: project_id.clone(),
            title: "Test Project".to_string(),
            description: "A test".to_string(),
            video_path: "/tmp/test.mp4".to_string(),
            style: "technical".to_string(),
            languages: vec!["en".to_string()],
            primary_language: "en".to_string(),
            frame_config: FrameConfig::default(),
            ai_config: AiConfig::default(),
            custom_prompt: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            edit_clips: None,
            timeline_effects: None,
            video_metadata: None,
            context_documents: None,
            edited_video_path: None,
            edited_video_plan_hash: None,
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        std::fs::write(project_dir.join("project.json"), &json).unwrap();

        // Verify we can read it back
        let loaded: ProjectConfig = serde_json::from_str(
            &std::fs::read_to_string(project_dir.join("project.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(loaded.title, "Test Project");
        assert_eq!(loaded.id, project_id);
    }

    #[test]
    fn test_default_styles() {
        let styles = default_styles();
        assert_eq!(styles.len(), 6);
        assert_eq!(styles[0].id, "executive");
        assert_eq!(styles[4].id, "training");
    }

    #[test]
    fn test_validate_project_id_valid_uuid() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(validate_project_id(&id).is_ok());
    }

    #[test]
    fn test_validate_project_id_rejects_invalid() {
        assert!(validate_project_id("not-a-uuid").is_err());
    }

    #[test]
    fn test_validate_project_id_rejects_path_traversal() {
        assert!(validate_project_id("../../etc/passwd").is_err());
    }

    #[test]
    fn test_find_next_version() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("v1_en.json"), "{}").unwrap();
        std::fs::write(temp.path().join("v2_en.json"), "{}").unwrap();

        assert_eq!(find_next_version(temp.path(), "en"), 3);
        assert_eq!(find_next_version(temp.path(), "ja"), 1);
    }

    #[test]
    fn test_validate_project_id_with_special_chars() {
        // Empty string
        assert!(validate_project_id("").is_err());
        // Spaces
        assert!(validate_project_id("hello world").is_err());
        // SQL injection attempt
        assert!(validate_project_id("'; DROP TABLE projects; --").is_err());
        // Null bytes
        assert!(validate_project_id("abc\0def").is_err());
        // Just dots
        assert!(validate_project_id("..").is_err());
        // Slash-based path traversal
        assert!(validate_project_id("/etc/passwd").is_err());
        // Backslash-based path traversal (Windows)
        assert!(validate_project_id("..\\..\\windows\\system32").is_err());
        // Unicode
        assert!(validate_project_id("\u{1F600}").is_err());
        // Partial UUID (too short)
        assert!(validate_project_id("550e8400-e29b-41d4").is_err());
        // Valid UUID should pass
        assert!(validate_project_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn test_default_styles_have_required_fields() {
        let styles = default_styles();
        assert!(!styles.is_empty());

        for style in &styles {
            assert!(
                !style.id.is_empty(),
                "Style id is empty for: {:?}",
                style.label
            );
            assert!(
                !style.label.is_empty(),
                "Style label is empty for: {}",
                style.id
            );
            assert!(
                !style.system_prompt.is_empty(),
                "Style system_prompt is empty for: {}",
                style.id
            );
            assert!(
                !style.description.is_empty(),
                "Style description is empty for: {}",
                style.id
            );
            assert!(
                !style.pacing.is_empty(),
                "Style pacing is empty for: {}",
                style.id
            );
        }

        // Verify all style IDs are unique
        let ids: Vec<&str> = styles.iter().map(|s| s.id.as_str()).collect();
        let unique_ids: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique_ids.len(), "Duplicate style IDs found");
    }

    // ── atomic_write ────────────────────────────────────────────────

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        atomic_write(&path, b"hello world").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "old content").unwrap();
        atomic_write(&path, b"new content").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn test_atomic_write_leaves_no_temp_file_on_success() {
        // After a successful atomic_write, there should be no sibling .tmp file left.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        atomic_write(&path, b"{}").unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let tmps: Vec<_> = entries.iter().filter(|n| n.contains(".tmp.")).collect();
        assert!(tmps.is_empty(), "leftover temp files: {tmps:?}");
    }

    #[test]
    fn test_atomic_write_preserves_old_when_new_fails() {
        // Writing to an invalid parent should fail without clobbering any file.
        let result = atomic_write(
            Path::new("/nonexistent-dir-for-narrator-tests/xyz/file.txt"),
            b"data",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_atomic_write_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.bin");
        let data: Vec<u8> = (0..=255u8).collect();
        atomic_write(&path, &data).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn test_atomic_write_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        atomic_write(&path, b"").unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn test_atomic_write_large_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let data = vec![0xAB; 2 * 1024 * 1024]; // 2MB
        atomic_write(&path, &data).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), data.len() as u64);
    }

    #[test]
    fn test_atomic_write_unicode_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unicode.txt");
        let text = "日本語 🎬 narration test";
        atomic_write(&path, text.as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    // ── Project CRUD roundtrip ────────────────────────────────────────
    //
    // These tests override `NARRATOR_DIR` to a tempdir so the real
    // `~/.narrator` is never touched. Env vars are process-wide, so we
    // serialize store-touching tests with a mutex and clean up via Drop so
    // a panicking test can't leak state to the next one.
    use std::sync::Mutex;
    static STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct NarratorDirGuard {
        _tempdir: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl NarratorDirGuard {
        fn new() -> Self {
            // Tolerate poisoned locks (a panicking test just means env state
            // was left dirty — we reset it below regardless).
            let lock = STORE_TEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let tempdir = tempfile::tempdir().unwrap();
            // SAFETY: we hold the lock, so no other test mutates env.
            unsafe {
                std::env::set_var("NARRATOR_DIR", tempdir.path());
            }
            Self {
                _tempdir: tempdir,
                _lock: lock,
            }
        }

        fn path(&self) -> &Path {
            self._tempdir.path()
        }
    }

    impl Drop for NarratorDirGuard {
        fn drop(&mut self) {
            // Always clear the env var, even on panic, before the lock
            // releases. Otherwise tests that don't use the guard (e.g.
            // `test_get_narrator_dir`) see a stale tempdir path.
            unsafe {
                std::env::remove_var("NARRATOR_DIR");
            }
        }
    }

    fn sample_config(id: &str, title: &str) -> ProjectConfig {
        ProjectConfig {
            schema_version: 1,
            id: id.into(),
            title: title.into(),
            description: "desc".into(),
            video_path: "/tmp/video.mp4".into(),
            style: "product_demo".into(),
            languages: vec!["en".into()],
            primary_language: "en".into(),
            frame_config: FrameConfig::default(),
            ai_config: AiConfig {
                provider: AiProviderKind::Claude,
                model: "claude-sonnet-4-20250514".into(),
                temperature: 0.7,
            },
            custom_prompt: String::new(),
            created_at: "2026-04-01T00:00:00Z".into(),
            updated_at: "2026-04-01T00:00:00Z".into(),
            edit_clips: None,
            timeline_effects: None,
            video_metadata: None,
            context_documents: None,
            edited_video_path: None,
            edited_video_plan_hash: None,
        }
    }

    fn sample_template(id: &str, name: &str) -> ProjectTemplate {
        ProjectTemplate {
            id: id.into(),
            name: name.into(),
            style: "product_demo".into(),
            languages: vec!["en".into()],
            primary_language: "en".into(),
            frame_config: FrameConfig::default(),
            ai_config: AiConfig {
                provider: AiProviderKind::Claude,
                model: "claude-sonnet-4-20250514".into(),
                temperature: 0.7,
            },
            custom_prompt: String::new(),
            tts_provider: "builtin".into(),
            created_at: "2026-04-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_project_create_load_list_roundtrip() {
        let g = NarratorDirGuard::new();
        let id = uuid::Uuid::new_v4().to_string();
        let cfg = sample_config(&id, "Roundtrip Project");
        create_project(&cfg).unwrap();

        let loaded = load_project(&id).unwrap();
        assert_eq!(loaded.title, "Roundtrip Project");
        assert_eq!(loaded.id, id);

        let list = list_projects().unwrap();
        assert!(list
            .iter()
            .any(|p| p.id == id && p.title == "Roundtrip Project"));
        drop(g);
    }

    #[test]
    fn test_project_save_updates_title() {
        let g = NarratorDirGuard::new();
        let id = uuid::Uuid::new_v4().to_string();
        let mut cfg = sample_config(&id, "Original");
        create_project(&cfg).unwrap();

        cfg.title = "Renamed".into();
        save_project(&cfg).unwrap();

        let loaded = load_project(&id).unwrap();
        assert_eq!(loaded.title, "Renamed");
        drop(g);
    }

    #[test]
    fn test_project_invalid_id_rejected() {
        let err = load_project("not-a-uuid").unwrap_err();
        assert!(err.to_string().contains("Invalid project ID"));
    }

    #[test]
    fn test_save_script_roundtrip_multi_version() {
        let g = NarratorDirGuard::new();
        let id = uuid::Uuid::new_v4().to_string();
        let cfg = sample_config(&id, "Scripts Test");
        create_project(&cfg).unwrap();

        let script = NarrationScript {
            title: "S1".into(),
            total_duration_seconds: 30.0,
            segments: vec![],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "m".into(),
                generated_at: "2026-04-01T00:00:00Z".into(),
            },
        };

        let path1 = save_script(&id, "en", &script).unwrap();
        let path2 = save_script(&id, "en", &script).unwrap();
        assert!(path1.contains("v1_en.json"));
        assert!(path2.contains("v2_en.json"));
        assert!(Path::new(&path1).exists());
        assert!(Path::new(&path2).exists());
        drop(g);
    }

    // ── Template CRUD ────────────────────────────────────────────────

    #[test]
    fn test_template_save_list_delete_cycle() {
        let g = NarratorDirGuard::new();
        let t1 = sample_template(&uuid::Uuid::new_v4().to_string(), "Template A");
        let t2 = sample_template(&uuid::Uuid::new_v4().to_string(), "Template B");
        save_template(&t1).unwrap();
        save_template(&t2).unwrap();

        let listed = list_templates().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|t| t.name == "Template A"));
        assert!(listed.iter().any(|t| t.name == "Template B"));

        delete_template(&t1.id).unwrap();
        let after = list_templates().unwrap();
        assert_eq!(after.len(), 1);
        assert!(after.iter().all(|t| t.id != t1.id));
        drop(g);
    }

    #[test]
    fn test_default_styles_shape() {
        let styles = default_styles();
        assert!(!styles.is_empty());
        for s in &styles {
            assert!(!s.id.is_empty());
            assert!(!s.label.is_empty());
            assert!(!s.system_prompt.is_empty());
        }
    }

    #[test]
    fn test_project_list_empty_when_no_projects() {
        let g = NarratorDirGuard::new();
        let list = list_projects().unwrap();
        assert_eq!(list.len(), 0);
        drop(g);
    }

    // ── Export/import (ZIP) round-trip ────────────────────────────────

    #[test]
    fn test_export_import_project_roundtrip() {
        let g1 = NarratorDirGuard::new();

        let id = uuid::Uuid::new_v4().to_string();
        let cfg = sample_config(&id, "Exportable");
        create_project(&cfg).unwrap();

        let script = NarrationScript {
            title: "s".into(),
            total_duration_seconds: 10.0,
            segments: vec![Segment {
                index: 0,
                start_seconds: 0.0,
                end_seconds: 5.0,
                text: "テスト".into(),
                visual_description: String::new(),
                emphasis: vec![],
                pace: Pace::Medium,
                pause_after_ms: 0,
                frame_refs: vec![],
                voice_override: None,
            }],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "m".into(),
                generated_at: "2026-04-01T00:00:00Z".into(),
            },
        };
        save_script(&id, "en", &script).unwrap();

        // Write a fake frame so export includes the frames dir
        let frames_dir = g1.path().join("projects").join(&id).join("frames");
        std::fs::write(frames_dir.join("frame_0.jpg"), b"\xFF\xD8\xFF\xE0fake").unwrap();

        let archive = g1.path().join("out.narrator");
        export_project(&id, &archive).unwrap();
        assert!(archive.exists());
        assert!(std::fs::metadata(&archive).unwrap().len() > 0);

        // Copy archive to a path outside g1's tempdir because we're about to
        // drop g1 and create a new guard (new tempdir, different NARRATOR_DIR).
        let shared_archive =
            std::env::temp_dir().join(format!("narrator-test-{}.narrator", uuid::Uuid::new_v4()));
        std::fs::copy(&archive, &shared_archive).unwrap();
        drop(g1);

        let g2 = NarratorDirGuard::new();
        let new_id = import_project(&shared_archive).unwrap();
        assert_ne!(new_id, id, "import should allocate a new id");

        let loaded = load_project(&new_id).unwrap();
        // import_project appends " (imported)" to the title to distinguish it
        assert!(
            loaded.title.starts_with("Exportable"),
            "unexpected imported title: {}",
            loaded.title
        );

        let full = load_project_full(&new_id).unwrap();
        let en = full.scripts.get("en").expect("en script should be present");
        assert_eq!(en.segments[0].text, "テスト");

        let new_frames = g2.path().join("projects").join(&new_id).join("frames");
        assert!(new_frames.join("frame_0.jpg").exists());
        drop(g2);

        // Clean up the shared archive outside tempdirs
        let _ = std::fs::remove_file(&shared_archive);
    }

    #[test]
    fn test_import_project_rejects_invalid_zip() {
        let g = NarratorDirGuard::new();
        let bogus = g.path().join("not-a-zip.narrator");
        std::fs::write(&bogus, b"this is not a zip").unwrap();
        let err = import_project(&bogus).unwrap_err();
        assert!(!err.to_string().is_empty());
        drop(g);
    }
}
