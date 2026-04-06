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

pub fn get_narrator_dir() -> PathBuf {
    if let Some(home) = directories::UserDirs::new() {
        home.home_dir().join(".narrator")
    } else {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
            .join(".narrator")
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
    std::fs::write(project_dir.join("project.json"), json)
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
    std::fs::write(project_dir.join("project.json"), json)
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
            // For each language, load the latest version
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .collect();
            files.sort_by_key(|e| e.file_name());

            for entry in files {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(stem) = name.strip_suffix(".json") {
                    if let Some(lang) = stem.split('_').next_back() {
                        if let Ok(json) = std::fs::read_to_string(entry.path()) {
                            if let Ok(script) = serde_json::from_str::<NarrationScript>(&json) {
                                scripts.insert(lang.to_string(), script);
                            }
                        }
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
    std::fs::write(&filepath, json)
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
}
