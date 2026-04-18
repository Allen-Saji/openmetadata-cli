//! `omd glossary assign <fqn> --term <term-fqn>` — attach glossary terms.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::entity;
use serde_json::{json, Value};

#[derive(clap::Subcommand, Debug)]
pub enum Action {
    /// Attach one or more glossary terms to an entity.
    Assign(AssignArgs),
}

#[derive(clap::Args, Debug)]
pub struct AssignArgs {
    /// Fully-qualified name of the target entity.
    pub fqn: String,

    /// Glossary term FQN (repeatable). Example: `CustomerData.Email`.
    #[arg(long = "term", required = true)]
    pub terms: Vec<String>,

    /// Entity type hint (resolved via search if omitted).
    #[arg(long)]
    pub r#type: Option<String>,

    /// Print the computed JSON patch without sending it.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(profile: &str, action: Action, ctx: &OutputCtx) -> CliResult<()> {
    match action {
        Action::Assign(args) => assign(profile, args, ctx).await,
    }
}

async fn assign(profile: &str, args: AssignArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let (entity_type, current) =
        entity::fetch_by_fqn(&client, &args.fqn, args.r#type.as_deref(), Some("tags")).await?;
    let id = entity::entity_id(&current)
        .ok_or_else(|| CliError::NotFound(format!("no id on entity {}", args.fqn)))?;

    let new_tags = merge_glossary_terms(&current, &args.terms);
    let op = if current.get("tags").is_some() {
        "replace"
    } else {
        "add"
    };
    let patch = json!([{ "op": op, "path": "/tags", "value": new_tags }]);

    if args.dry_run {
        output::print_json(&patch);
        return Ok(());
    }

    let path = format!("{}/{}", entity::endpoint_for_type(&entity_type), id);
    let updated = client.json_patch(&path, &patch).await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&updated);
    } else {
        output::success(format!(
            "assigned {} glossary term(s) to {} {}",
            args.terms.len(),
            entity_type,
            args.fqn
        ));
    }
    Ok(())
}

fn merge_glossary_terms(current: &Value, terms: &[String]) -> Vec<Value> {
    let mut tags: Vec<Value> = current
        .get("tags")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    for term_fqn in terms {
        if tags.iter().any(|t| {
            t.get("tagFQN")
                .and_then(|v| v.as_str())
                .map(|s| s == term_fqn)
                .unwrap_or(false)
        }) {
            continue;
        }
        tags.push(json!({
            "tagFQN": term_fqn,
            "source": "Glossary",
            "labelType": "Manual",
            "state": "Confirmed",
        }));
    }
    tags
}
