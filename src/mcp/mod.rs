//! MCP server implementation.
//!
//! Exposes a curated set of OpenMetadata operations as MCP tools so AI agents
//! can drive the catalog directly. Not a 1:1 map of the REST API — tools are
//! tuned for LLM usage (search, describe, lineage, edit, CSV, quality).

pub mod tools;

use crate::error::CliResult;
use rmcp::{transport::stdio, ServiceExt};

/// Launch the stdio MCP server and block until the client disconnects.
pub async fn serve_stdio() -> CliResult<()> {
    let service = tools::OmdMcp::new()
        .serve(stdio())
        .await
        .map_err(|e| crate::error::CliError::Other(anyhow::anyhow!("mcp serve: {e}")))?;
    service
        .waiting()
        .await
        .map_err(|e| crate::error::CliError::Other(anyhow::anyhow!("mcp wait: {e}")))?;
    Ok(())
}
