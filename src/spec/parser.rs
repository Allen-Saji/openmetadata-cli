use crate::error::CliResult;
use std::path::{Path, PathBuf};

/// Path where the cached OpenAPI spec lives.
pub fn cache_path() -> CliResult<PathBuf> {
    if let Ok(home) = std::env::var("OMD_HOME") {
        return Ok(PathBuf::from(home).join("spec.json"));
    }
    let base = dirs::home_dir()
        .ok_or_else(|| crate::error::CliError::Config("no home directory".into()))?;
    Ok(base.join(".omd").join("spec.json"))
}

/// Load the cached OpenAPI spec if present.
#[allow(dead_code)]
pub fn load_cached() -> CliResult<Option<serde_json::Value>> {
    let p = cache_path()?;
    load_from(&p)
}

#[allow(dead_code)]
pub fn load_from(p: &Path) -> CliResult<Option<serde_json::Value>> {
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(p)?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    Ok(Some(v))
}

pub fn save_cache(spec: &serde_json::Value) -> CliResult<()> {
    let p = cache_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(spec)?;
    std::fs::write(&p, text)?;
    Ok(())
}
