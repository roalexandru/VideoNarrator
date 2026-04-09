//! Secure credential storage using OS keychain.
//! macOS: Keychain, Windows: Credential Manager, Linux: Secret Service (libsecret).

use crate::error::NarratorError;

const SERVICE_NAME: &str = "com.narrator.app";

/// Store a secret in the OS keychain.
pub fn set_secret(key: &str, value: &str) -> Result<(), NarratorError> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .map_err(|e| NarratorError::AuthError(format!("Keyring init error: {e}")))?;
    entry
        .set_password(value)
        .map_err(|e| NarratorError::AuthError(format!("Failed to store key: {e}")))?;
    Ok(())
}

/// Retrieve a secret from the OS keychain.
pub fn get_secret(key: &str) -> Result<Option<String>, NarratorError> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .map_err(|e| NarratorError::AuthError(format!("Keyring init error: {e}")))?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            tracing::warn!("Keyring read failed for {key}: {e}");
            Ok(None) // Graceful fallback - don't crash if keychain is locked
        }
    }
}

/// Delete a secret from the OS keychain.
#[allow(dead_code)]
pub fn delete_secret(key: &str) -> Result<(), NarratorError> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .map_err(|e| NarratorError::AuthError(format!("Keyring init error: {e}")))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // Already gone
        Err(e) => Err(NarratorError::AuthError(format!(
            "Failed to delete key: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyring_roundtrip() {
        // Verify the keyring can write and read back a value on this platform
        let test_key = "test_roundtrip_key";
        let test_value = "test_value_12345";

        // Write
        let write_result = set_secret(test_key, test_value);
        assert!(write_result.is_ok(), "Keyring write failed: {:?}", write_result.err());

        // Read
        let read_result = get_secret(test_key);
        assert!(read_result.is_ok(), "Keyring read failed: {:?}", read_result.err());
        assert_eq!(read_result.unwrap(), Some(test_value.to_string()));

        // Cleanup
        let _ = delete_secret(test_key);
    }


}

/// Migrate keys from a plaintext config HashMap to the keychain.
/// Returns the number of keys migrated.
pub fn migrate_from_plaintext(keys: &std::collections::HashMap<String, String>) -> usize {
    let mut migrated = 0;
    for (key_name, key_value) in keys {
        if key_value.is_empty() {
            continue;
        }
        let keychain_key = format!("api_key_{key_name}");
        match set_secret(&keychain_key, key_value) {
            Ok(()) => {
                migrated += 1;
                tracing::info!("Migrated {key_name} API key to OS keychain");
            }
            Err(e) => {
                tracing::warn!("Failed to migrate {key_name} to keychain: {e}");
            }
        }
    }
    migrated
}
