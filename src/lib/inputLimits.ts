/**
 * Character limits for free-text fields submitted to `generate_narration`.
 *
 * Mirrors the validation in `src-tauri/src/commands.rs::generate_narration`
 * (search for "characters or fewer"). Both sides must agree — otherwise the
 * frontend lets the user type something the backend will reject after a
 * multi-minute apply-edits run, which is exactly the bug this module exists
 * to prevent. Update both files in the same commit.
 */
export const TITLE_MAX_CHARS = 500;
export const DESCRIPTION_MAX_CHARS = 5000;
export const CUSTOM_PROMPT_MAX_CHARS = 10000;
