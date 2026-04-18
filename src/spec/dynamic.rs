//! Build clap commands at runtime from an [`Index`] of OpenAPI operations.
//!
//! Each tag becomes a subcommand group; each operation becomes an action.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::spec::index::{Index, Operation, ParamKind};
use crate::spec::parser;
use crate::spec::request::{self, BuiltRequest};
use clap::{Arg, ArgAction, Command};

/// Leak a runtime string to `&'static str` so clap can intern it.
/// Bounded lifetime: one group's arg metadata per invocation.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Render an after-help footer listing dynamic groups from the cached spec.
///
/// Returns an empty string if no spec is cached — the hint tells the user to sync.
pub fn after_help() -> String {
    let spec = match parser::load_cached() {
        Ok(Some(s)) => s,
        _ => {
            return "Dynamic commands (from OpenAPI): run `omd configure` + `omd sync` to enable."
                .to_string();
        }
    };
    let idx = Index::from_spec(&spec);
    if idx.groups.is_empty() {
        return String::new();
    }
    let groups: Vec<&str> = idx.groups().into_iter().collect();
    let preview: Vec<&str> = groups.iter().take(12).copied().collect();
    let more = if groups.len() > preview.len() {
        format!(", ... ({} more)", groups.len() - preview.len())
    } else {
        String::new()
    };
    format!(
        "Dynamic commands (from OpenAPI, {} groups): {}{}\nRun `omd <group> --help` to explore.",
        groups.len(),
        preview.join(", "),
        more
    )
}

/// Load the cached spec and index it, or return a friendly error.
pub fn load_index() -> CliResult<Index> {
    let spec = parser::load_cached()?.ok_or_else(|| {
        CliError::Config("no cached OpenAPI spec; run `omd sync` first (or `omd configure`)".into())
    })?;
    Ok(Index::from_spec(&spec))
}

/// Build a clap::Command for a single tag group.
pub fn build_group_command(group: &str, ops: &[Operation]) -> Command {
    let mut cmd = Command::new(leak(group.to_string()))
        .about(leak(format!("{} endpoints (from OpenAPI spec)", group)))
        .subcommand_required(true)
        .arg_required_else_help(true);
    for op in ops {
        cmd = cmd.subcommand(build_action_command(op));
    }
    cmd
}

fn build_action_command(op: &Operation) -> Command {
    let mut about = op
        .summary
        .clone()
        .unwrap_or_else(|| format!("{} {}", op.method, op.path));
    if about.len() > 120 {
        about.truncate(117);
        about.push_str("...");
    }
    let mut cmd = Command::new(leak(op.action.clone())).about(leak(about));

    for p in &op.path_params {
        let id = leak(request::path_arg_id(&p.name));
        let help_text = p
            .description
            .clone()
            .unwrap_or_else(|| format!("path parameter `{}`", p.name));
        let arg = Arg::new(id)
            .required(true)
            .value_name(leak(p.name.to_uppercase()))
            .help(leak(help_text));
        cmd = cmd.arg(arg);
    }

    for p in &op.query_params {
        let id = leak(request::query_arg_id(&p.name));
        let help_text = p
            .description
            .clone()
            .unwrap_or_else(|| format!("query parameter `{}`", p.name));
        let mut arg = Arg::new(id)
            .long(leak(p.name.clone()))
            .required(p.required)
            .help(leak(help_text));
        arg = match p.kind {
            ParamKind::Array => arg
                .action(ArgAction::Append)
                .value_name(leak(p.name.to_uppercase())),
            ParamKind::Boolean => arg.value_name("BOOL").value_parser(["true", "false"]),
            ParamKind::Integer | ParamKind::Number => arg.value_name("N"),
            ParamKind::String => arg.value_name(leak(p.name.to_uppercase())),
        };
        cmd = cmd.arg(arg);
    }

    if op.has_body {
        cmd = cmd.arg(
            Arg::new("body")
                .long("body")
                .value_name("BODY")
                .required(op.body_required)
                .help("JSON body: inline `{...}`, `@path/to/file.json`, or `-` for stdin"),
        );
    }

    cmd
}

/// Dispatch a dynamic `omd <group> <action> [args]` invocation.
pub async fn dispatch(profile: &str, ctx: &OutputCtx, args: Vec<String>) -> CliResult<()> {
    if args.is_empty() {
        return Err(CliError::InvalidInput("expected a subcommand".into()));
    }
    let group = args[0].clone();
    let idx = load_index()?;
    let ops = idx.get(&group).ok_or_else(|| unknown_group(&idx, &group))?;

    let mut argv: Vec<String> = vec!["omd".into(), group.clone()];
    argv.extend(args.into_iter().skip(1));

    let root = Command::new("omd").subcommand(build_group_command(&group, ops));
    let top_matches = root.try_get_matches_from(argv).map_err(clap_err)?;
    let group_matches = top_matches
        .subcommand_matches(&group)
        .ok_or_else(|| CliError::InvalidInput(format!("expected group `{group}`")))?;
    let (action, action_matches) = group_matches
        .subcommand()
        .ok_or_else(|| CliError::InvalidInput(format!("missing action for `{group}`")))?;

    let op = ops
        .iter()
        .find(|o| o.action == action)
        .ok_or_else(|| CliError::InvalidInput(format!("unknown action `{group} {action}`")))?;

    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;
    let req: BuiltRequest = request::build(op, action_matches)?;
    let resp = client
        .json(req.method, &req.path, &req.query, req.body)
        .await?;

    output::render_api_response(&resp, ctx);
    Ok(())
}

fn clap_err(e: clap::Error) -> CliError {
    // Mirror clap's native behavior: help/version → stdout + exit 0; errors → stderr.
    let kind = e.kind();
    let rendered = e.render().to_string();
    match kind {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            print!("{rendered}");
            std::process::exit(0);
        }
        _ => {
            eprint!("{rendered}");
            CliError::InvalidInput("argument parse failed".into())
        }
    }
}

fn unknown_group(idx: &Index, group: &str) -> CliError {
    let available: Vec<&str> = idx.groups().into_iter().take(12).collect();
    let more = if idx.groups.len() > available.len() {
        format!(" ... ({} more)", idx.groups.len() - available.len())
    } else {
        String::new()
    };
    CliError::InvalidInput(format!(
        "unknown group `{group}`. available: {}{more}",
        available.join(", ")
    ))
}
