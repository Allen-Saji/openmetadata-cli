//! `omd export <type> <fqn> [-o FILE]` — download entity metadata as CSV.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::{csv, entity};
use reqwest::Method;
use std::io::Write;
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    /// Entity type (table, database, databaseSchema, glossary, glossaryTerm,
    /// team, user, databaseService, securityService, driveService, testCase).
    pub r#type: String,

    /// Fully-qualified name of the entity to export.
    pub fqn: String,

    /// Write CSV to this file. Omit to print to stdout.
    #[arg(short = 'o', long = "out")]
    pub out: Option<PathBuf>,
}

pub async fn run(profile: &str, args: ExportArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let collection = csv::collection_for(&args.r#type)?;
    let path = format!(
        "{collection}/name/{}/export",
        entity::urlencode_segment(&args.fqn)
    );

    let v = client.json(Method::GET, &path, &[], None).await?;
    let text = v
        .as_str()
        .ok_or_else(|| CliError::Api {
            status: 0,
            message: "unexpected non-string response from export endpoint".into(),
        })?
        .to_string();

    match args.out {
        Some(path) => {
            let mut f = std::fs::File::create(&path)?;
            f.write_all(text.as_bytes())?;
            if output::pretty(ctx) {
                let bytes = text.len();
                let rows = text.lines().count().saturating_sub(1);
                output::success(format!(
                    "wrote {rows} row(s), {bytes} bytes to {}",
                    path.display()
                ));
            }
        }
        None => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}
