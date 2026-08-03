//! Narration preferences that survive regeneration.
//!
//! `refine_segment` and `refine_script` apply the user's instruction to the text
//! and then discard the instruction. Nothing recorded that the user said "stop
//! saying seamlessly" or "always keep the error-handling section" — so pressing
//! Regenerate threw away every preference they had expressed, and they re-taught
//! the model from scratch. Iteration restarted instead of compounding.
//!
//! This keeps an append-only list per project. Accepted entries are injected into
//! the system prompt on subsequent generations, so the next draft already honours
//! what the user asked for last time.
//!
//! ## Why entries are editable and deletable
//!
//! A preference is a standing instruction, and standing instructions go stale —
//! "keep it under two minutes" stops applying when the video is recut. A log the
//! user cannot prune would slowly poison every future generation with advice
//! that no longer fits, so the store supports removal and the UI is expected to
//! expose it.

use crate::error::NarratorError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// File name inside the project directory.
const FILE: &str = "preferences.json";

/// Cap on stored entries.
///
/// Prompt budget is finite and the oldest instruction is the least likely to
/// still reflect what the user wants. Trimming keeps the most recent.
pub const MAX_ENTRIES: usize = 40;

/// Longest single instruction kept, in characters.
///
/// A refine instruction can be a paragraph; as a standing preference only the
/// gist is useful, and an unbounded string could crowd out the rest of the
/// prompt.
pub const MAX_INSTRUCTION_CHARS: usize = 240;

/// Where a preference came from, so the UI can explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceSource {
    /// Derived from a whole-script refine instruction.
    ScriptRefinement,
    /// Derived from a single-segment refine instruction.
    SegmentRefinement,
    /// Typed directly by the user as a standing instruction.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    /// Stable id so the UI can delete a specific entry.
    pub id: String,
    /// The instruction, as the user phrased it.
    pub instruction: String,
    pub source: PreferenceSource,
    /// False when the user dismissed it. Kept rather than deleted so the same
    /// instruction is not re-suggested after being rejected once.
    #[serde(default = "default_true")]
    pub active: bool,
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreferenceStore {
    #[serde(default)]
    pub entries: Vec<Preference>,
}

/// Normalize an instruction for storage and comparison.
///
/// Collapses whitespace and truncates. Returns `None` for anything that carries
/// no standing instruction — empty text, or a bare "do it" with no content.
pub fn normalize_instruction(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() < 3 {
        return None;
    }
    let truncated: String = trimmed.chars().take(MAX_INSTRUCTION_CHARS).collect();
    Some(truncated)
}

/// True when two instructions say the same thing for dedup purposes.
///
/// Case- and punctuation-insensitive, because "Stop saying seamlessly." and
/// "stop saying seamlessly" are one preference, and storing both would double
/// its weight in the prompt.
fn same_instruction(a: &str, b: &str) -> bool {
    let key = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    key(a) == key(b)
}

impl PreferenceStore {
    /// Record an instruction, or refresh an existing equivalent one.
    ///
    /// Returns the id of the stored entry, or `None` when the instruction carried
    /// nothing worth keeping.
    pub fn record(
        &mut self,
        raw: &str,
        source: PreferenceSource,
        now: &str,
        id: &str,
    ) -> Option<String> {
        let instruction = normalize_instruction(raw)?;

        // Repeating an instruction is a signal it still matters, so move the
        // existing entry to the end and re-activate it rather than adding a
        // duplicate that would count twice in the prompt.
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| same_instruction(&e.instruction, &instruction))
        {
            let mut existing = self.entries.remove(pos);
            existing.active = true;
            existing.created_at = now.to_string();
            let existing_id = existing.id.clone();
            self.entries.push(existing);
            return Some(existing_id);
        }

        self.entries.push(Preference {
            id: id.to_string(),
            instruction,
            source,
            active: true,
            created_at: now.to_string(),
        });
        self.trim();
        Some(id.to_string())
    }

    /// Drop the oldest entries once over budget.
    fn trim(&mut self) {
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..excess);
        }
    }

    /// Deactivate an entry without forgetting it, so it is not re-suggested.
    pub fn deactivate(&mut self, id: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.active = false;
                true
            }
            None => false,
        }
    }

    /// Remove an entry outright.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn active(&self) -> impl Iterator<Item = &Preference> {
        self.entries.iter().filter(|e| e.active)
    }

    /// The prompt block injected into subsequent generations.
    ///
    /// Empty when there is nothing active, so a fresh project's prompt is
    /// unchanged. Ordered oldest-first, matching the order the user established
    /// them in.
    pub fn prompt_block(&self) -> String {
        let active: Vec<&Preference> = self.active().collect();
        if active.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = active
            .iter()
            .map(|p| format!("• {}", p.instruction))
            .collect();
        format!(
            "\n\n## ESTABLISHED PREFERENCES\n\n\
             The user has already given this feedback on earlier drafts of this \
             project's narration. Honour it in this draft without being asked \
             again:\n\n{}\n",
            lines.join("\n")
        )
    }
}

/// Path to a project's preference file.
pub fn path_for(project_dir: &Path) -> PathBuf {
    project_dir.join(FILE)
}

/// Load a project's preferences.
///
/// A missing or unreadable file yields an empty store: losing preferences is a
/// degraded experience, but failing to load a project over them would be worse.
pub fn load(project_dir: &Path) -> PreferenceStore {
    let path = path_for(project_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(
                "preferences at {} unreadable ({e}), starting empty",
                path.display()
            );
            PreferenceStore::default()
        }),
        Err(_) => PreferenceStore::default(),
    }
}

/// Persist a project's preferences.
pub fn save(project_dir: &Path, store: &PreferenceStore) -> Result<(), NarratorError> {
    std::fs::create_dir_all(project_dir)
        .map_err(|e| NarratorError::ProjectError(format!("preferences dir: {e}")))?;
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| NarratorError::SerializationError(e.to_string()))?;
    std::fs::write(path_for(project_dir), json)
        .map_err(|e| NarratorError::ProjectError(format!("write preferences: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn store_with(instructions: &[&str]) -> PreferenceStore {
        let mut store = PreferenceStore::default();
        for (i, text) in instructions.iter().enumerate() {
            store.record(
                text,
                PreferenceSource::ScriptRefinement,
                NOW,
                &format!("id{i}"),
            );
        }
        store
    }

    // ── Normalization ───────────────────────────────────────────────────

    #[test]
    fn normalization_collapses_whitespace() {
        assert_eq!(
            normalize_instruction("  stop   saying\n\tseamlessly  ").unwrap(),
            "stop saying seamlessly"
        );
    }

    #[test]
    fn normalization_rejects_content_free_instructions() {
        // An empty or near-empty instruction is not a standing preference.
        for empty in ["", "   ", "\n", "ok"] {
            assert!(
                normalize_instruction(empty).is_none(),
                "{empty:?} should not become a preference"
            );
        }
    }

    #[test]
    fn normalization_truncates_a_long_instruction() {
        let long = "a ".repeat(500);
        let normalized = normalize_instruction(&long).unwrap();
        assert!(normalized.chars().count() <= MAX_INSTRUCTION_CHARS);
    }

    // ── Recording and dedup ─────────────────────────────────────────────

    #[test]
    fn recording_keeps_the_instruction_verbatim_after_normalizing() {
        let store = store_with(&["Never use the word robust"]);
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].instruction, "Never use the word robust");
        assert!(store.entries[0].active);
    }

    #[test]
    fn recording_the_same_instruction_twice_does_not_duplicate_it() {
        // Two identical entries would double the instruction's weight in the
        // prompt.
        let store = store_with(&[
            "Stop saying seamlessly.",
            "stop saying seamlessly",
            "  STOP SAYING SEAMLESSLY  ",
        ]);
        assert_eq!(
            store.entries.len(),
            1,
            "case and punctuation must not split"
        );
    }

    #[test]
    fn repeating_an_instruction_reactivates_it() {
        // The user dismissed it, then asked again — asking again wins.
        let mut store = store_with(&["Keep the error handling section"]);
        let id = store.entries[0].id.clone();
        assert!(store.deactivate(&id));
        assert_eq!(store.active().count(), 0);

        store.record(
            "keep the error handling section",
            PreferenceSource::SegmentRefinement,
            NOW,
            "new-id",
        );
        assert_eq!(store.entries.len(), 1, "still one entry");
        assert_eq!(store.active().count(), 1, "and it is active again");
    }

    #[test]
    fn recording_a_content_free_instruction_stores_nothing() {
        let mut store = PreferenceStore::default();
        assert!(store
            .record("  ", PreferenceSource::Manual, NOW, "x")
            .is_none());
        assert!(store.entries.is_empty());
    }

    #[test]
    fn store_trims_to_the_cap_keeping_the_newest() {
        let instructions: Vec<String> = (0..MAX_ENTRIES + 10)
            .map(|i| format!("rule number {i}"))
            .collect();
        let refs: Vec<&str> = instructions.iter().map(String::as_str).collect();
        let store = store_with(&refs);
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        // The oldest are the ones dropped.
        assert!(store.entries[0].instruction.contains("rule number 10"));
        assert!(store
            .entries
            .last()
            .unwrap()
            .instruction
            .contains(&format!("rule number {}", MAX_ENTRIES + 9)));
    }

    // ── Removal ─────────────────────────────────────────────────────────

    #[test]
    fn deactivate_keeps_the_entry_but_drops_it_from_the_prompt() {
        let mut store = store_with(&["Avoid the word powerful"]);
        let id = store.entries[0].id.clone();
        store.deactivate(&id);
        assert_eq!(store.entries.len(), 1, "kept, so it is not re-suggested");
        assert!(store.prompt_block().is_empty());
    }

    #[test]
    fn remove_deletes_outright() {
        let mut store = store_with(&["Avoid the word powerful"]);
        let id = store.entries[0].id.clone();
        assert!(store.remove(&id));
        assert!(store.entries.is_empty());
        assert!(!store.remove("nonexistent"));
    }

    #[test]
    fn deactivating_an_unknown_id_is_reported() {
        let mut store = store_with(&["something"]);
        assert!(!store.deactivate("not-a-real-id"));
    }

    // ── Prompt block ────────────────────────────────────────────────────

    #[test]
    fn prompt_block_is_empty_for_a_fresh_project() {
        // A new project's prompt must be byte-identical to before this feature.
        assert!(PreferenceStore::default().prompt_block().is_empty());
    }

    #[test]
    fn prompt_block_lists_active_entries_oldest_first() {
        let store = store_with(&["First rule", "Second rule", "Third rule"]);
        let block = store.prompt_block();
        assert!(block.contains("ESTABLISHED PREFERENCES"));
        let first = block.find("First rule").unwrap();
        let second = block.find("Second rule").unwrap();
        let third = block.find("Third rule").unwrap();
        assert!(first < second && second < third, "order not preserved");
        // Must tell the model not to ask again, or it re-litigates every draft.
        assert!(block.contains("without being asked again"));
    }

    #[test]
    fn prompt_block_excludes_deactivated_entries() {
        let mut store = store_with(&["Keep this one", "Drop this one"]);
        let drop_id = store.entries[1].id.clone();
        store.deactivate(&drop_id);
        let block = store.prompt_block();
        assert!(block.contains("Keep this one"));
        assert!(!block.contains("Drop this one"));
    }

    // ── Persistence ─────────────────────────────────────────────────────

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with(&["Rule one", "Rule two"]);
        save(dir.path(), &store).unwrap();

        let loaded = load(dir.path());
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].instruction, "Rule one");
        assert_eq!(loaded.entries[1].source, PreferenceSource::ScriptRefinement);
        assert!(loaded.entries[0].active);
    }

    #[test]
    fn loading_a_project_without_preferences_yields_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).entries.is_empty());
    }

    #[test]
    fn a_corrupt_file_degrades_to_empty_rather_than_failing() {
        // Losing preferences is bad; failing to open the project is worse.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_for(dir.path()), b"{ not json").unwrap();
        assert!(load(dir.path()).entries.is_empty());
    }

    #[test]
    fn entries_saved_before_the_active_flag_existed_load_as_active() {
        // Forward-compat: an older file has no `active` key.
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "entries": [{
                "id": "old",
                "instruction": "legacy rule",
                "source": "manual",
                "created_at": NOW
            }]
        });
        std::fs::write(path_for(dir.path()), legacy.to_string()).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.active().count(), 1, "must default to active");
    }
}
