use crate::error::{CliError, CliResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const CONFIG_FILE: &str = "config.toml";
const CREDENTIALS_FILE: &str = "credentials";

fn config_dir() -> CliResult<PathBuf> {
    if let Ok(home) = std::env::var("OMD_HOME") {
        return Ok(PathBuf::from(home));
    }
    let base = dirs::home_dir().ok_or_else(|| CliError::Config("no home directory".into()))?;
    Ok(base.join(".omd"))
}

fn ensure_dir() -> CliResult<PathBuf> {
    let dir = config_dir()?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Profile {
    pub host: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            host: None,
            timeout_secs: default_timeout(),
        }
    }
}

fn default_timeout() -> u64 {
    60
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl ConfigFile {
    pub fn load() -> CliResult<Self> {
        let path = config_dir()?.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self) -> CliResult<()> {
        let path = ensure_dir()?.join(CONFIG_FILE);
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, text)?;
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_string()).or_default()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

impl Credentials {
    pub fn load() -> CliResult<Self> {
        let path = config_dir()?.join(CREDENTIALS_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self) -> CliResult<()> {
        let path = ensure_dir()?.join(CREDENTIALS_FILE);
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, &text)?;
        // Restrict permissions to owner-only (0600) on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&path)?.permissions();
            perm.set_mode(0o600);
            fs::set_permissions(&path, perm)?;
        }
        Ok(())
    }

    pub fn token(&self, profile: &str) -> Option<&str> {
        self.tokens.get(profile).map(|s| s.as_str())
    }

    pub fn set_token(&mut self, profile: &str, token: String) {
        self.tokens.insert(profile.to_string(), token);
    }

    pub fn clear(&mut self, profile: &str) {
        self.tokens.remove(profile);
    }
}

/// Fully resolved configuration for a given profile, with env overrides applied.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub profile: String,
    pub host: String,
    pub token: Option<String>,
    pub timeout_secs: u64,
}

impl ResolvedConfig {
    pub fn load(profile: &str) -> CliResult<Self> {
        let file = ConfigFile::load()?;
        let creds = Credentials::load()?;
        let p = file.profile(profile).cloned().unwrap_or_default();

        let host = std::env::var("OMD_HOST")
            .ok()
            .or(p.host)
            .ok_or(CliError::NotConfigured)?;
        let token = std::env::var("OMD_TOKEN")
            .ok()
            .or_else(|| creds.token(profile).map(|s| s.to_string()));

        Ok(Self {
            profile: profile.to_string(),
            host: host.trim_end_matches('/').to_string(),
            token,
            timeout_secs: p.timeout_secs,
        })
    }

    pub fn require_token(&self) -> CliResult<&str> {
        self.token.as_deref().ok_or(CliError::NotAuthenticated)
    }
}

pub fn paths_summary() -> CliResult<(PathBuf, PathBuf)> {
    let dir = config_dir()?;
    Ok((dir.join(CONFIG_FILE), dir.join(CREDENTIALS_FILE)))
}
