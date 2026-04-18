//! `omd mcp` — start the Model Context Protocol server on stdio.

use crate::error::CliResult;
use crate::mcp;
use crate::output::OutputCtx;

#[derive(clap::Args, Debug)]
pub struct McpArgs {}

pub async fn run(_args: McpArgs, _ctx: &OutputCtx) -> CliResult<()> {
    mcp::serve_stdio().await
}
