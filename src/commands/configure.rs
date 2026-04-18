use crate::config::{paths_summary, ConfigFile};
use crate::error::{CliError, CliResult};
use crate::output;
use crate::output::OutputCtx;
use clap::Subcommand;
use dialoguer::Input;

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Set a configuration value (host, timeout)
    Set { key: String, value: String },
    /// Get a configuration value
    Get { key: String },
    /// List current configuration
    List,
}

pub async fn run(profile: &str, action: Option<Action>, ctx: &OutputCtx) -> CliResult<()> {
    match action {
        None => interactive(profile, ctx),
        Some(Action::Set { key, value }) => set(profile, &key, &value, ctx),
        Some(Action::Get { key }) => get(profile, &key, ctx),
        Some(Action::List) => list(profile, ctx),
    }
}

fn interactive(profile: &str, ctx: &OutputCtx) -> CliResult<()> {
    let mut file = ConfigFile::load()?;
    let current = file.profile(profile).cloned().unwrap_or_default();

    let host: String = Input::new()
        .with_prompt("OpenMetadata host URL")
        .with_initial_text(current.host.unwrap_or_default())
        .interact_text()
        .map_err(|e| CliError::InvalidInput(e.to_string()))?;

    let timeout: u64 = Input::new()
        .with_prompt("Request timeout (seconds)")
        .default(current.timeout_secs)
        .interact_text()
        .map_err(|e| CliError::InvalidInput(e.to_string()))?;

    let p = file.profile_mut(profile);
    p.host = Some(host.trim_end_matches('/').to_string());
    p.timeout_secs = timeout;
    file.save()?;

    let (cfg_path, _) = paths_summary()?;
    if output::pretty(ctx) {
        output::success(format!(
            "saved profile '{}' to {}",
            profile,
            cfg_path.display()
        ));
        output::info("next: run `omd auth login` to save an API token");
    } else {
        output::print_json(&serde_json::json!({
            "profile": profile,
            "config_file": cfg_path.display().to_string(),
            "saved": true,
        }));
    }
    Ok(())
}

fn set(profile: &str, key: &str, value: &str, ctx: &OutputCtx) -> CliResult<()> {
    let mut file = ConfigFile::load()?;
    let p = file.profile_mut(profile);
    match key {
        "host" => p.host = Some(value.trim_end_matches('/').to_string()),
        "timeout" | "timeout_secs" => {
            p.timeout_secs = value.parse().map_err(|_| {
                CliError::InvalidInput(format!("timeout must be u64, got '{value}'"))
            })?;
        }
        _ => return Err(CliError::InvalidInput(format!("unknown key '{key}'"))),
    }
    file.save()?;
    if output::pretty(ctx) {
        output::success(format!("set {key}={value} for profile '{profile}'"));
    } else {
        output::print_json(&serde_json::json!({ "profile": profile, "key": key, "value": value }));
    }
    Ok(())
}

fn get(profile: &str, key: &str, ctx: &OutputCtx) -> CliResult<()> {
    let file = ConfigFile::load()?;
    let p = file.profile(profile).cloned().unwrap_or_default();
    let value = match key {
        "host" => p.host.unwrap_or_default(),
        "timeout" | "timeout_secs" => p.timeout_secs.to_string(),
        _ => return Err(CliError::InvalidInput(format!("unknown key '{key}'"))),
    };
    if output::pretty(ctx) {
        println!("{value}");
    } else {
        output::print_json(&serde_json::json!({ "profile": profile, "key": key, "value": value }));
    }
    Ok(())
}

fn list(profile: &str, ctx: &OutputCtx) -> CliResult<()> {
    let file = ConfigFile::load()?;
    let p = file.profile(profile).cloned().unwrap_or_default();
    let (cfg_path, creds_path) = paths_summary()?;
    if output::pretty(ctx) {
        output::print_kv(&[
            ("profile", profile.to_string()),
            ("host", p.host.clone().unwrap_or_else(|| "(unset)".into())),
            ("timeout_secs", p.timeout_secs.to_string()),
            ("config_file", cfg_path.display().to_string()),
            ("credentials_file", creds_path.display().to_string()),
        ]);
    } else {
        output::print_json(&serde_json::json!({
            "profile": profile,
            "host": p.host,
            "timeout_secs": p.timeout_secs,
            "config_file": cfg_path.display().to_string(),
            "credentials_file": creds_path.display().to_string(),
        }));
    }
    Ok(())
}
