//! omd - Command-line tool for OpenMetadata.

mod auth;
mod client;
mod commands;
mod config;
mod error;
mod output;
mod spec;

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
}

#[tokio::main]
async fn main() {
    init_tracing();
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli).await {
        output::render_error(&e);
        std::process::exit(e.exit_code());
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("OMD_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
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
    }
}
