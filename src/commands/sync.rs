use crate::config::ResolvedConfig;
use crate::error::CliResult;
use crate::output;
use crate::spec::parser;
use crate::output::OutputCtx;
#[derive(clap::Args, Debug)]
pub struct SyncArgs {
    /// Fetch from a custom URL instead of the configured host's /swagger.json
    #[arg(long)]
    pub from: Option<String>,
}

pub async fn run(profile: &str, args: SyncArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    let url = match args.from {
        Some(u) => u,
        None => format!("{}/swagger.json", cfg.host),
    };

    output::info(format!("fetching spec from {url}"));

    let http = reqwest::Client::builder()
        .user_agent(concat!("omd/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut req = http.get(&url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(crate::error::CliError::Api {
            status: status.as_u16(),
            message: resp.text().await.unwrap_or_default(),
        });
    }
    let spec: serde_json::Value = resp.json().await?;
    parser::save_cache(&spec)?;

    let title = spec.get("info").and_then(|i| i.get("title")).and_then(|s| s.as_str()).unwrap_or("(unknown)");
    let version = spec.get("info").and_then(|i| i.get("version")).and_then(|s| s.as_str()).unwrap_or("?");
    let paths = spec.get("paths").and_then(|p| p.as_object()).map(|o| o.len()).unwrap_or(0);

    if output::pretty(ctx) {
        output::success(format!("cached {title} v{version} ({paths} paths)"));
        let cache = parser::cache_path()?;
        output::info(format!("cache file: {}", cache.display()));
    } else {
        output::print_json(&serde_json::json!({
            "title": title,
            "version": version,
            "paths": paths,
            "cache_file": parser::cache_path()?.display().to_string(),
        }));
    }
    Ok(())
}
