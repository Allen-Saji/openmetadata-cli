use crate::auth::jwt;
use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output;
use crate::output::OutputCtx;
use clap::Subcommand;
use dialoguer::Password;

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Save a JWT bearer token for the current profile
    Login {
        /// Token value. If omitted, prompt interactively (input is hidden)
        #[arg(long)]
        token: Option<String>,
    },
    /// Show current authentication status
    Status,
    /// Remove the saved token
    Logout,
}

pub async fn run(profile: &str, action: Action, ctx: &OutputCtx) -> CliResult<()> {
    match action {
        Action::Login { token } => login(profile, token, ctx).await,
        Action::Status => status(profile, ctx).await,
        Action::Logout => logout(profile, ctx),
    }
}

async fn login(profile: &str, token: Option<String>, ctx: &OutputCtx) -> CliResult<()> {
    let token = match token {
        Some(t) => t,
        None => Password::new()
            .with_prompt("Paste JWT token (input hidden)")
            .interact()
            .map_err(|e| CliError::InvalidInput(e.to_string()))?,
    };
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(CliError::InvalidInput("empty token".into()));
    }
    jwt::save_token(profile, trimmed)?;

    if output::pretty(ctx) {
        output::success(format!("saved token for profile '{profile}'"));
        output::info("verify with `omd auth status`");
    } else {
        output::print_json(&serde_json::json!({ "profile": profile, "saved": true }));
    }
    Ok(())
}

async fn status(profile: &str, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = match ResolvedConfig::load(profile) {
        Ok(c) => c,
        Err(CliError::NotConfigured) => {
            return err_report(ctx, "not_configured", "run `omd configure` first");
        }
        Err(e) => return Err(e),
    };

    if cfg.token.is_none() {
        return err_report(ctx, "not_authenticated", "run `omd auth login`");
    }

    // Verify with a lightweight call
    let client = OmdClient::new(&cfg)?;
    match client.get_json("v1/users/loggedInUser").await {
        Ok(v) => {
            let name = v
                .get("name")
                .and_then(|s| s.as_str())
                .unwrap_or("<unknown>")
                .to_string();
            let email = v.get("email").and_then(|s| s.as_str()).unwrap_or("").to_string();
            if output::pretty(ctx) {
                output::print_kv(&[
                    ("profile", cfg.profile.clone()),
                    ("host", cfg.host.clone()),
                    ("user", name),
                    ("email", email),
                    ("status", "authenticated".into()),
                ]);
            } else {
                output::print_json(&serde_json::json!({
                    "profile": cfg.profile,
                    "host": cfg.host,
                    "user": name,
                    "email": email,
                    "authenticated": true,
                }));
            }
            Ok(())
        }
        Err(e) => {
            if output::pretty(ctx) {
                output::warn(format!("token rejected: {e}"));
                output::info("token may be expired - run `omd auth login` again");
            } else {
                output::print_json(&serde_json::json!({
                    "profile": cfg.profile,
                    "host": cfg.host,
                    "authenticated": false,
                    "error": e.to_string(),
                }));
            }
            Err(e)
        }
    }
}

fn logout(profile: &str, ctx: &OutputCtx) -> CliResult<()> {
    jwt::clear_token(profile)?;
    if output::pretty(ctx) {
        output::success(format!("cleared token for profile '{profile}'"));
    } else {
        output::print_json(&serde_json::json!({ "profile": profile, "cleared": true }));
    }
    Ok(())
}

fn err_report(ctx: &OutputCtx, kind: &str, hint: &str) -> CliResult<()> {
    if output::pretty(ctx) {
        output::warn(hint);
    } else {
        output::print_json(&serde_json::json!({
            "authenticated": false,
            "kind": kind,
            "hint": hint,
        }));
    }
    Err(match kind {
        "not_configured" => CliError::NotConfigured,
        _ => CliError::NotAuthenticated,
    })
}
