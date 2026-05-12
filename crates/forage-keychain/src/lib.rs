//! Cross-platform secret storage backed by the `keyring` crate.
//!
//! macOS Keychain / Windows Credential Manager / Linux Secret Service —
//! same API everywhere. Used by `forage-http` (session-cache AES-GCM key)
//! and `forage-hub` (OAuth tokens, API keys).

use thiserror::Error;

pub const SERVICE_CLI: &str = "com.foragelang.cli";
pub const SERVICE_STUDIO: &str = "com.foragelang.studio";

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
}

pub fn read_secret(service: &str, account: &str) -> Result<Option<String>, KeychainError> {
    let entry = keyring::Entry::new(service, account)?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn write_secret(service: &str, account: &str, value: &str) -> Result<(), KeychainError> {
    let entry = keyring::Entry::new(service, account)?;
    entry.set_password(value)?;
    Ok(())
}

pub fn delete_secret(service: &str, account: &str) -> Result<(), KeychainError> {
    let entry = keyring::Entry::new(service, account)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip test against the real keychain. Skipped on CI / when
    /// the platform doesn't have a desktop session (Linux without
    /// secret-service running). Marked ignored by default; run with
    /// `cargo test -p forage-keychain -- --ignored` to exercise it.
    #[test]
    #[ignore = "requires desktop keychain"]
    fn round_trips() {
        let service = "com.foragelang.test";
        let account = "forage-keychain-test";
        let value = "secret-value-42";
        write_secret(service, account, value).unwrap();
        let got = read_secret(service, account).unwrap();
        assert_eq!(got.as_deref(), Some(value));
        delete_secret(service, account).unwrap();
        let got = read_secret(service, account).unwrap();
        assert_eq!(got, None);
    }
}
