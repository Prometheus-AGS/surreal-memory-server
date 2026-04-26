//! Regenerate committed MCP and REST contract JSON files.

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    write_json(
        "docs/specs/mcp-tools.json",
        &surreal_memory_server::contracts::mcp_tools_spec(),
    )?;
    write_json(
        "docs/specs/rest-api.json",
        &surreal_memory_server::contracts::rest_api_spec(),
    )?;
    Ok(())
}

fn write_json(path: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value).context("serialize contract spec")?;
    std::fs::write(path, format!("{json}\n"))
        .with_context(|| format!("write contract spec to {path}"))
}
