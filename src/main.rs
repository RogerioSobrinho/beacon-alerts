use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(
    name = "beacon",
    version,
    about = "Distributed event and alert notification system"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the central event server.
    Server(ServerArgs),
    /// Start a local agent with a durable event spool.
    Agent(AgentArgs),
    /// Create and inspect a normalized event payload.
    Send(SendArgs),
    /// Inspect events waiting in a local spool.
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
struct ServerArgs {
    /// Listen address. Networking is intentionally not implemented in the bootstrap.
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,
}

#[derive(Debug, Args)]
struct AgentArgs {
    /// Beacon server endpoint.
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    server: String,
    /// Local durable spool directory.
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
}

#[derive(Debug, Args)]
struct SendArgs {
    #[arg(long)]
    event_type: String,
    #[arg(long)]
    source: String,
    #[arg(long)]
    host: String,
    #[arg(long, value_enum)]
    state: EventState,
    #[arg(long, value_enum)]
    severity: Severity,
    #[arg(long)]
    fingerprint: String,
    /// JSON object containing allowlisted event facts.
    #[arg(long, default_value = "{}")]
    facts: String,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum EventState {
    Firing,
    Resolved,
    Info,
}

#[derive(Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Deserialize, Serialize)]
struct Event {
    schema_version: u16,
    event_id: String,
    event_type: String,
    source: String,
    host_id: String,
    state: EventState,
    severity: Severity,
    fingerprint: String,
    occurred_at: String,
    facts: BTreeMap<String, serde_json::Value>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Server(args) => {
            println!("Beacon server bootstrap; bind={}", args.bind);
            println!(
                "Networking, persistence, authentication, and delivery are not implemented yet."
            );
        }
        Command::Agent(args) => {
            println!(
                "Beacon agent bootstrap; server={}; spool={}",
                args.server,
                args.spool.display()
            );
            println!("Local durable spool and authenticated transport are not implemented yet.");
        }
        Command::Send(args) => print_event(args)?,
        Command::Replay(args) => {
            println!("Beacon replay bootstrap; spool={}", args.spool.display());
            println!("Spool inspection and delivery replay are not implemented yet.");
        }
    }

    Ok(())
}

fn print_event(args: SendArgs) -> Result<()> {
    let facts = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&args.facts)
        .context("--facts must be a JSON object")?;
    let event = Event {
        schema_version: 1,
        event_id: "local-bootstrap-event".to_string(),
        event_type: args.event_type,
        source: args.source,
        host_id: args.host,
        state: args.state,
        severity: args.severity,
        fingerprint: args.fingerprint,
        occurred_at: "1970-01-01T00:00:00Z".to_string(),
        facts,
    };

    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}
