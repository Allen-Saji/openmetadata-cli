use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output;
use crate::output::OutputCtx;
use reqwest::Method;

#[derive(clap::Args, Debug)]
pub struct RawArgs {
    /// HTTP method (GET, POST, PUT, PATCH, DELETE)
    pub method: String,
    /// Path (e.g. "v1/tables" or "/api/v1/tables")
    pub path: String,
    /// Query param (repeatable). Use key=value
    #[arg(long = "query", short = 'q')]
    pub query: Vec<String>,
    /// Body JSON. Prefix with @ to read from file (e.g. --body @payload.json)
    #[arg(long)]
    pub body: Option<String>,
}

pub async fn run(profile: &str, args: RawArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let method = Method::from_bytes(args.method.to_uppercase().as_bytes())
        .map_err(|_| CliError::InvalidInput(format!("invalid method '{}'", args.method)))?;

    let mut pairs: Vec<(String, String)> = Vec::new();
    for q in &args.query {
        let (k, v) = q
            .split_once('=')
            .ok_or_else(|| CliError::InvalidInput(format!("query must be key=value, got '{q}'")))?;
        pairs.push((k.to_string(), v.to_string()));
    }

    let body = match &args.body {
        None => None,
        Some(s) if s.starts_with('@') => {
            let path = &s[1..];
            let text = std::fs::read_to_string(path)?;
            Some(serde_json::from_str::<serde_json::Value>(&text)?)
        }
        Some(s) => Some(serde_json::from_str::<serde_json::Value>(s)?),
    };

    let v = client.json(method, &args.path, &pairs, body).await?;
    output::print_json(&v);
    let _ = ctx; // JSON output is always appropriate for raw responses
    Ok(())
}
