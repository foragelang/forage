//! On-disk auth store at `~/Library/Forage/Auth/<host>.json` (chmod 600).
//!
//! Holds OAuth access + refresh tokens, the signed-in GitHub login, and
//! the host the tokens are scoped to. The keychain holds nothing here —
//! tokens are JWTs anyway, and the directory's chmod 600 keeps other
//! users on a shared machine from reading them.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::HubResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub login: String,
    pub hub_url: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct AuthStore {
    root: PathBuf,
}

impl AuthStore {
    /// Default root: `~/Library/Forage/Auth/` on macOS,
    /// `$XDG_DATA_HOME/forage/auth/` on Linux, `%APPDATA%\Forage\Auth\`
    /// on Windows.
    pub fn default_root() -> PathBuf {
        if cfg!(target_os = "macos") {
            if let Some(home) = dirs::home_dir() {
                return home.join("Library").join("Forage").join("Auth");
            }
        }
        if cfg!(target_os = "windows") {
            if let Some(data) = dirs::data_dir() {
                return data.join("Forage").join("Auth");
            }
        }
        if let Some(data) = dirs::data_dir() {
            return data.join("forage").join("auth");
        }
        PathBuf::from(".forage-auth")
    }

    pub fn new() -> Self {
        Self {
            root: Self::default_root(),
        }
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn file_for(&self, host: &str) -> PathBuf {
        let safe = host.replace(['/', '\\'], "_");
        self.root.join(format!("{safe}.json"))
    }

    pub fn read(&self, host: &str) -> HubResult<Option<AuthTokens>> {
        let path = self.file_for(host);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let t: AuthTokens = serde_json::from_str(&raw)?;
        Ok(Some(t))
    }

    pub fn write(&self, tokens: &AuthTokens) -> HubResult<()> {
        let host = host_of(&tokens.hub_url);
        fs::create_dir_all(&self.root)?;
        let path = self.file_for(&host);
        let raw = serde_json::to_string_pretty(tokens)?;
        write_chmod_600(&path, raw.as_bytes())?;
        Ok(())
    }

    pub fn delete(&self, host: &str) -> HubResult<()> {
        let path = self.file_for(host);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }
}

impl Default for AuthStore {
    fn default() -> Self {
        Self::new()
    }
}

fn host_of(url: &str) -> String {
    let after_scheme = url.split("//").nth(1).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or(after_scheme);
    host.to_string()
}

fn write_chmod_600(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_tokens() {
        let tmp = TempDir::new().unwrap();
        let store = AuthStore::with_root(tmp.path().to_path_buf());
        let tokens = AuthTokens {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            login: "dima".into(),
            hub_url: "https://api.foragelang.com".into(),
            issued_at: 1,
            expires_at: 3600,
        };
        store.write(&tokens).unwrap();
        let got = store.read("api.foragelang.com").unwrap();
        assert_eq!(got, Some(tokens.clone()));
        store.delete("api.foragelang.com").unwrap();
        let got = store.read("api.foragelang.com").unwrap();
        assert_eq!(got, None);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_chmod_600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let store = AuthStore::with_root(tmp.path().to_path_buf());
        let tokens = AuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            login: "x".into(),
            hub_url: "https://api.example.com".into(),
            issued_at: 0,
            expires_at: 0,
        };
        store.write(&tokens).unwrap();
        let path = store.file_for("api.example.com");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
