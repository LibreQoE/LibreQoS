mod client;
mod context;
mod mutating;
mod openapi;
mod suites;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[command(name = "api_test", about = "LibreQoS API verification harness", version)]
struct Cli {
    /// API base URL; the API is mounted by lqosd on this path
    #[arg(long, env = "API_BASE", default_value = "http://127.0.0.1:9123/api/v3")]
    base: String,

    /// Local API key for `x-bearer` authentication (env `LQOS_API_KEY`)
    #[arg(long, env = "LQOS_API_KEY")]
    api_key: Option<String>,

    /// Request timeout (seconds)
    #[arg(long, default_value_t = 15)]
    timeout_secs: u64,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Quick read-only probe of a small set of endpoints
    Smoke,
    /// Read-only verification of all GET endpoints in OpenAPI
    Full,
    /// State-changing verification (requires --yes). Automatically backs up and restores files.
    Mutating {
        /// Confirm you want to run mutating tests on this node.
        #[arg(long)]
        yes: bool,
    },
}

/// Resolves the `x-bearer` key, accepting `LICENSE_KEY` for one release.
///
/// The standalone API service called this a licence key; the daemon's mount
/// takes a local API key, so the variable was renamed and the old one warns.
fn resolve_api_key(from_cli: Option<String>) -> Result<String> {
    if let Some(key) = from_cli.filter(|key| !key.trim().is_empty()) {
        return Ok(key);
    }
    if let Ok(legacy) = std::env::var("LICENSE_KEY")
        && !legacy.trim().is_empty()
    {
        eprintln!("warning: LICENSE_KEY is deprecated; set LQOS_API_KEY instead");
        return Ok(legacy);
    }
    anyhow::bail!("an API key is required: set LQOS_API_KEY (or pass --api-key)")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let api_key = resolve_api_key(cli.api_key)?;
    let client = client::ApiClient::new(cli.base, api_key, cli.timeout_secs)?;

    match cli.cmd {
        Command::Smoke => suites::run_smoke(&client).await,
        Command::Full => suites::run_full(&client).await,
        Command::Mutating { yes } => mutating::run_mutating(&client, yes).await,
    }
}
