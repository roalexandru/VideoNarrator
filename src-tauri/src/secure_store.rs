//! Secure credential storage using OS keychain.
//! macOS: Keychain, Windows: Credential Manager, Linux: Secret Service (libsecret).
//!
//! All secrets are stored in a **single** keychain entry as a JSON map.
//! After the initial read, everything is served from an in-memory cache so the
//! OS keychain is only touched once for reads (and once per write).
//!
//! On macOS debug builds, reads use the `security` CLI to avoid the keychain
//! password prompt that appears for unsigned binaries.

use crate::error::NarratorError;
use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE_NAME: &str = "com.narrator.app";
const ACCOUNT_NAME: &str = "secrets";

/// In-memory cache — populated once at first access, then never re-read from keychain.
static CACHE: std::sync::LazyLock<Mutex<Option<HashMap<String, String>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Read the bundled secrets from keychain using the `security` CLI.
/// This avoids the macOS keychain password prompt on unsigned (debug) binaries
/// because `security` is a signed Apple binary with its own keychain access.
#[cfg(all(debug_assertions, target_os = "macos"))]
fn read_keychain() -> HashMap<String, String> {
    match std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            SERVICE_NAME,
            "-a",
            ACCOUNT_NAME,
            "-w",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let json = String::from_utf8_lossy(&output.stdout).trim().to_string();
            serde_json::from_str(&json).unwrap_or_default()
        }
        _ => HashMap::new(),
    }
}

/// Write the bundled secrets to keychain using the `security` CLI.
#[cfg(all(debug_assertions, target_os = "macos"))]
fn write_keychain(map: &HashMap<String, String>) -> Result<(), NarratorError> {
    let json = serde_json::to_string(map)
        .map_err(|e| NarratorError::AuthError(format!("Failed to serialize secrets: {e}")))?;

    // Delete existing entry first (add-generic-password -U can fail on some macOS versions)
    let _ = std::process::Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            SERVICE_NAME,
            "-a",
            ACCOUNT_NAME,
        ])
        .output();

    let output = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            SERVICE_NAME,
            "-a",
            ACCOUNT_NAME,
            "-w",
            &json,
            "-T",
            "", // Allow access from any app (avoids ACL issues in dev)
        ])
        .output()
        .map_err(|e| NarratorError::AuthError(format!("Failed to run security CLI: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::AuthError(format!(
            "security add-generic-password failed: {stderr}"
        )));
    }

    Ok(())
}

/// Read/write via `keyring` crate (used in release builds, or non-macOS debug).
#[cfg(not(all(debug_assertions, target_os = "macos")))]
fn read_keychain() -> HashMap<String, String> {
    match keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME) {
        Ok(entry) => match entry.get_password() {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(keyring::Error::NoEntry) => HashMap::new(),
            Err(e) => {
                tracing::warn!("Keychain read failed: {e}");
                HashMap::new()
            }
        },
        Err(e) => {
            tracing::warn!("Keyring init error: {e}");
            HashMap::new()
        }
    }
}

#[cfg(not(all(debug_assertions, target_os = "macos")))]
fn write_keychain(map: &HashMap<String, String>) -> Result<(), NarratorError> {
    let json = serde_json::to_string(map)
        .map_err(|e| NarratorError::AuthError(format!("Failed to serialize secrets: {e}")))?;

    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME)
        .map_err(|e| NarratorError::AuthError(format!("Keyring init error: {e}")))?;
    entry
        .set_password(&json)
        .map_err(|e| NarratorError::AuthError(format!("Failed to store secrets: {e}")))?;

    Ok(())
}

/// Read the full secrets map (from cache or keychain).
fn read_all() -> HashMap<String, String> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref map) = *cache {
        return map.clone();
    }

    let map = read_keychain();
    *cache = Some(map.clone());
    map
}

/// Write the full secrets map to keychain and update cache.
fn write_all(map: &HashMap<String, String>) -> Result<(), NarratorError> {
    write_keychain(map)?;
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(map.clone());
    Ok(())
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Store a secret.
pub fn set_secret(key: &str, value: &str) -> Result<(), NarratorError> {
    let mut map = read_all();
    map.insert(key.to_string(), value.to_string());
    write_all(&map)
}

/// Retrieve a secret.
pub fn get_secret(key: &str) -> Result<Option<String>, NarratorError> {
    let map = read_all();
    Ok(map.get(key).cloned())
}

/// Delete a secret.
#[allow(dead_code)]
pub fn delete_secret(key: &str) -> Result<(), NarratorError> {
    let mut map = read_all();
    if map.remove(key).is_some() {
        write_all(&map)?;
    }
    Ok(())
}

/// Migrate keys from a plaintext config HashMap to secure storage.
/// Returns the number of keys migrated.
pub fn migrate_from_plaintext(keys: &HashMap<String, String>) -> usize {
    let mut map = read_all();
    let mut migrated = 0;
    for (key_name, key_value) in keys {
        if key_value.is_empty() {
            continue;
        }
        let keychain_key = format!("api_key_{key_name}");
        if let std::collections::hash_map::Entry::Vacant(e) = map.entry(keychain_key) {
            e.insert(key_value.clone());
            migrated += 1;
            tracing::info!("Migrated {key_name} API key to secure storage");
        }
    }
    if migrated > 0 {
        if let Err(e) = write_all(&map) {
            tracing::warn!("Failed to write migrated keys: {e}");
            return 0;
        }
    }
    migrated
}

/// Migrate old per-key keychain entries into the single bundled entry.
/// In macOS debug builds, uses the `security` CLI to read old entries without prompting.
/// In release builds, uses the `keyring` crate (signed binary = no prompt).
pub fn migrate_old_keychain_entries() {
    let old_keys = [
        "api_key_claude",
        "api_key_openai",
        "api_key_gemini",
        "api_key_elevenlabs",
        "api_key_azure_tts",
    ];

    let mut map = read_all();
    let mut migrated = 0;

    for key in &old_keys {
        if map.contains_key(*key) {
            continue; // Already migrated
        }

        let value = read_old_entry(key);
        if let Some(v) = value {
            if !v.is_empty() {
                map.insert(key.to_string(), v);
                delete_old_entry(key);
                migrated += 1;
            }
        }
    }

    if migrated > 0 {
        if let Err(e) = write_all(&map) {
            tracing::warn!("Failed to write migrated old keychain entries: {e}");
        } else {
            tracing::info!("Migrated {migrated} old keychain entries to bundled format");
        }
    }
}

/// Read an old per-key entry from the keychain.
#[cfg(all(debug_assertions, target_os = "macos"))]
fn read_old_entry(key: &str) -> Option<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", SERVICE_NAME, "-a", key, "-w"])
        .output()
        .ok()?;
    if output.status.success() {
        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if val.is_empty() {
            None
        } else {
            Some(val)
        }
    } else {
        None
    }
}

#[cfg(all(debug_assertions, target_os = "macos"))]
fn delete_old_entry(key: &str) {
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE_NAME, "-a", key])
        .output();
}

#[cfg(not(all(debug_assertions, target_os = "macos")))]
fn read_old_entry(key: &str) -> Option<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, key).ok()?;
    match entry.get_password() {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(not(all(debug_assertions, target_os = "macos")))]
fn delete_old_entry(key: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, key) {
        let _ = entry.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires system keychain — not available in CI
    fn test_keyring_roundtrip() {
        let test_key = "test_roundtrip_key";
        let test_value = "test_value_12345";

        let write_result = set_secret(test_key, test_value);
        assert!(
            write_result.is_ok(),
            "Keyring write failed: {:?}",
            write_result.err()
        );

        let read_result = get_secret(test_key);
        assert!(
            read_result.is_ok(),
            "Keyring read failed: {:?}",
            read_result.err()
        );
        assert_eq!(read_result.unwrap(), Some(test_value.to_string()));

        let _ = delete_secret(test_key);
    }
}
