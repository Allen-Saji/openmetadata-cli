use crate::auth::{jwt, sso};
use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output;
use crate::output::OutputCtx;
use clap::Subcommand;
use dialoguer::Password;
use reqwest::Method;

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Save a JWT bearer token for the current profile
    Login {
        /// Token value. If omitted, prompt interactively (input is hidden)
        #[arg(long)]
        token: Option<String>,

        /// Log in via SSO (OIDC authorization code with PKCE). Opens a browser.
        #[arg(long, conflicts_with = "token")]
        sso: bool,

        /// Override the OIDC client id (for orgs that register a separate public CLI client).
        #[arg(long, requires = "sso")]
        client_id: Option<String>,

        /// Override the OIDC authority (issuer) URL.
        #[arg(long, requires = "sso")]
        authority: Option<String>,

        /// Scopes to request (space-separated). Defaults to "openid email profile".
        #[arg(long, requires = "sso")]
        scopes: Option<String>,
    },
    /// Show current authentication status
    Status,
    /// Remove the saved token
    Logout,
}

pub async fn run(profile: &str, action: Action, ctx: &OutputCtx) -> CliResult<()> {
    match action {
        Action::Login {
            token,
            sso,
            client_id,
            authority,
            scopes,
        } => {
            if sso {
                sso_login(profile, client_id, authority, scopes, ctx).await
            } else {
                jwt_login(profile, token, ctx).await
            }
        }
        Action::Status => status(profile, ctx).await,
        Action::Logout => logout(profile, ctx),
    }
}

async fn jwt_login(profile: &str, token: Option<String>, ctx: &OutputCtx) -> CliResult<()> {
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

async fn sso_login(
    profile: &str,
    client_id_override: Option<String>,
    authority_override: Option<String>,
    scopes_override: Option<String>,
    ctx: &OutputCtx,
) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    let client = OmdClient::new(&cfg)?;

    // 1. Discover OM server auth config (provider, authority, clientId).
    let om_auth = client
        .json(Method::GET, "v1/system/config/auth", &[], None)
        .await?;
    let mut om_auth = sso::OmAuthConfig::from_value(&om_auth)?;
    if let Some(c) = client_id_override {
        om_auth.client_id = c;
    }
    if let Some(a) = authority_override {
        om_auth.authority = a;
    }
    if output::pretty(ctx) {
        output::info(format!(
            "SSO provider: {} ({})",
            om_auth.provider, om_auth.authority
        ));
    }

    // 2. OIDC discovery.
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        om_auth.authority.trim_end_matches('/')
    );
    let discovery: serde_json::Value = reqwest::Client::new()
        .get(&discovery_url)
        .send()
        .await
        .map_err(CliError::Http)?
        .error_for_status()
        .map_err(CliError::Http)?
        .json()
        .await
        .map_err(CliError::Http)?;
    let endpoints = sso::OidcEndpoints::from_value(&discovery)?;

    // 3. PKCE + loopback listener.
    let pkce = sso::Pkce::generate();
    let state = sso::random_state();
    let (listener, redirect_uri) = sso::bind_loopback()?;
    let scopes = scopes_override.as_deref().unwrap_or(sso::DEFAULT_SCOPES);
    let authorize_url = sso::build_authorize_url(
        &endpoints,
        &om_auth.client_id,
        &redirect_uri,
        scopes,
        &state,
        &pkce,
    )?;

    if output::pretty(ctx) {
        output::info("opening browser...");
        output::info(format!("(if it doesn't open, visit): {authorize_url}"));
    }
    let _ = webbrowser::open(&authorize_url);

    // 4. Wait for callback on the loopback socket.
    let params = sso::accept_callback(listener, sso::CALLBACK_TIMEOUT)?;
    if let Some(err) = params.error {
        let desc = params.error_description.unwrap_or_default();
        return Err(CliError::Other(anyhow::anyhow!(
            "provider returned error: {err} ({desc})"
        )));
    }
    if params.state.as_deref() != Some(state.as_str()) {
        return Err(CliError::Other(anyhow::anyhow!(
            "state mismatch on callback — possible CSRF, aborted"
        )));
    }
    let code = params
        .code
        .ok_or_else(|| CliError::Other(anyhow::anyhow!("no code in callback")))?;

    // 5. Exchange code for token.
    let token_resp: serde_json::Value = reqwest::Client::new()
        .post(&endpoints.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &om_auth.client_id),
            ("code_verifier", &pkce.verifier),
        ])
        .send()
        .await
        .map_err(CliError::Http)?
        .error_for_status()
        .map_err(CliError::Http)?
        .json()
        .await
        .map_err(CliError::Http)?;

    let token = token_resp
        .get("id_token")
        .and_then(|v| v.as_str())
        .or_else(|| token_resp.get("access_token").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            CliError::Other(anyhow::anyhow!(
                "token response missing both id_token and access_token"
            ))
        })?;

    jwt::save_token(profile, token)?;

    if output::pretty(ctx) {
        output::success(format!("SSO login complete for profile '{profile}'"));
        output::info("verify with `omd auth status`");
    } else {
        output::print_json(&serde_json::json!({
            "profile": profile,
            "saved": true,
            "provider": om_auth.provider,
        }));
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
            let email = v
                .get("email")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
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
