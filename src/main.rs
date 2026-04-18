//! omd - Command-line tool for OpenMetadata.

mod auth;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod spec;
mod util;

use clap::{Parser, Subcommand};
use error::CliResult;
use output::OutputCtx;

#[derive(Parser)]
#[command(name = "omd")]
#[command(author = "Allen Saji <allensaji04@gmail.com>")]
#[command(version)]
#[command(about = "Command-line tool for OpenMetadata", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Configuration profile to use
    #[arg(long, global = true, default_value = "default", env = "OMD_PROFILE")]
    profile: String,

    /// Force JSON output (NDJSON for lists)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure host and settings
    Configure {
        #[command(subcommand)]
        action: Option<commands::configure::Action>,
    },

    /// Authentication (login, status, logout)
    Auth {
        #[command(subcommand)]
        action: commands::auth::Action,
    },

    /// Refresh the cached OpenAPI spec
    Sync(commands::sync::SyncArgs),

    /// Search the catalog
    Search(commands::search::SearchArgs),

    /// Describe an entity by FQN
    Describe(commands::describe::DescribeArgs),

    /// Show entity lineage (tree, mermaid, dot, or json)
    Lineage(commands::lineage::LineageArgs),

    /// Update entity fields (description, display name, owner, tier)
    Edit(commands::edit::EditArgs),

    /// Add or remove classification tags on an entity
    Tag(commands::tag::TagArgs),

    /// Glossary operations (assign terms)
    Glossary {
        #[command(subcommand)]
        action: commands::glossary::Action,
    },

    /// Data quality: list, results, latest
    Quality {
        #[command(subcommand)]
        action: commands::quality::Action,
    },

    /// Generate a shell completion script
    Completions(commands::completions::CompletionsArgs),

    /// Raw HTTP request against the OpenMetadata API
    Raw(commands::raw::RawArgs),

    /// Dynamic OpenAPI-backed commands: `omd <group> <action> [args]`
    #[command(external_subcommand)]
    Dynamic(Vec<String>),
}

#[tokio::main]
async fn main() {
    init_tracing();
    let matches = <Cli as clap::CommandFactory>::command()
        .after_help(spec::dynamic::after_help())
        .get_matches();
    let cli = match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(64);
        }
    };
    if let Err(e) = dispatch(cli).await {
        output::render_error(&e);
        std::process::exit(e.exit_code());
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("OMD_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

async fn dispatch(cli: Cli) -> CliResult<()> {
    let ctx = OutputCtx { json: cli.json };
    match cli.command {
        Commands::Configure { action } => {
            commands::configure::run(&cli.profile, action, &ctx).await
        }
        Commands::Auth { action } => commands::auth::run(&cli.profile, action, &ctx).await,
        Commands::Sync(args) => commands::sync::run(&cli.profile, args, &ctx).await,
        Commands::Search(args) => commands::search::run(&cli.profile, args, &ctx).await,
        Commands::Describe(args) => commands::describe::run(&cli.profile, args, &ctx).await,
        Commands::Lineage(args) => commands::lineage::run(&cli.profile, args, &ctx).await,
        Commands::Edit(args) => commands::edit::run(&cli.profile, args, &ctx).await,
        Commands::Tag(args) => commands::tag::run(&cli.profile, args, &ctx).await,
        Commands::Glossary { action } => commands::glossary::run(&cli.profile, action, &ctx).await,
        Commands::Quality { action } => commands::quality::run(&cli.profile, action, &ctx).await,
        Commands::Completions(args) => commands::completions::run::<Cli>(args),
        Commands::Raw(args) => commands::raw::run(&cli.profile, args, &ctx).await,
        Commands::Dynamic(args) => spec::dynamic::dispatch(&cli.profile, &ctx, args).await,
    }
}
