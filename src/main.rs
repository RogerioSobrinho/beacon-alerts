#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use beacon_alerts::agent::{drain, AgentConfig};
use beacon_alerts::model::{Event, EventState, Severity};
use beacon_alerts::policy::PolicyCatalog;
use beacon_alerts::server::{serve, ServerConfig};
use beacon_alerts::spool::Spool;
use chrono::{SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

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
    /// Deliver queued events to the central server.
    Agent(AgentArgs),
    /// Create and queue a normalized event payload.
    Send(SendArgs),
    /// Inspect events waiting in a local spool.
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
struct ServerArgs {
    /// Listen address.
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,
    /// Directory where accepted events are durably stored.
    #[arg(long, default_value = "/var/lib/beacon/events")]
    data: PathBuf,
    /// File containing the bearer token required by the event intake endpoint.
    #[arg(long, default_value = "/etc/beacon/server.token")]
    token_file: PathBuf,
    /// JSON file containing event-to-channel notification policies.
    #[arg(long)]
    policy_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct AgentArgs {
    /// Beacon server endpoint.
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    server: String,
    /// Local durable spool directory.
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
    /// File containing the bearer token used to authenticate to the server.
    #[arg(long, default_value = "/etc/beacon/agent.token")]
    token_file: PathBuf,
    /// Maximum number of events to attempt in this run.
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Maximum attempts per event, including the first submission.
    #[arg(long, default_value_t = 3)]
    max_attempts: usize,
    /// Delay between failed attempts.
    #[arg(long, default_value_t = 2)]
    retry_delay_seconds: u64,
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
    /// Local spool directory. The event is durably queued unless --print-only is set.
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
    /// Render the event without writing it to the local spool.
    #[arg(long)]
    print_only: bool,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Server(args) => {
            serve(ServerConfig {
                bind: args.bind,
                data: args.data,
                token: read_token(&args.token_file)?,
                policy: read_policy(args.policy_file.as_deref())?,
            })
            .await?;
        }
        Command::Agent(args) => {
            let delivered = drain(AgentConfig {
                server: args.server,
                spool: args.spool,
                token: read_token(&args.token_file)?,
                limit: args.limit,
                max_attempts: args.max_attempts,
                retry_delay_seconds: args.retry_delay_seconds,
            })
            .await?;
            println!("delivered {delivered} event(s)");
        }
        Command::Send(args) => send_event(args)?,
        Command::Replay(args) => replay_events(args)?,
    }
    Ok(())
}

fn read_token(path: &std::path::Path) -> Result<String> {
    let token =
        fs::read_to_string(path).with_context(|| format!("read token file {}", path.display()))?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("token file {} is empty", path.display());
    }
    Ok(token)
}

fn read_policy(path: Option<&std::path::Path>) -> Result<PolicyCatalog> {
    let Some(path) = path else {
        return Ok(PolicyCatalog::default());
    };
    let policy: PolicyCatalog = serde_json::from_str(
        &fs::read_to_string(path)
            .with_context(|| format!("read policy file {}", path.display()))?,
    )
    .with_context(|| format!("parse policy file {}", path.display()))?;
    policy.validate()?;
    Ok(policy)
}

fn send_event(args: SendArgs) -> Result<()> {
    let facts = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&args.facts)
        .context("--facts must be a JSON object")?;
    let event = Event {
        schema_version: 1,
        event_id: Uuid::new_v4().to_string(),
        event_type: args.event_type,
        source: args.source,
        host_id: args.host,
        state: args.state,
        severity: args.severity,
        fingerprint: args.fingerprint,
        occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        facts,
    };
    event.validate()?;

    println!("{}", serde_json::to_string_pretty(&event)?);
    if !args.print_only {
        let path = Spool::open(args.spool)?.enqueue(&event)?;
        eprintln!("queued: {}", path.display());
    }
    Ok(())
}

fn replay_events(args: ReplayArgs) -> Result<()> {
    let spool = Spool::open(args.spool)?;
    let events = spool.list()?;
    println!("{} pending event(s)", events.len());
    for event in events {
        println!(
            "{} {} {} {}",
            event.event_id,
            event.state.as_str(),
            event.severity.as_str(),
            event.event_type
        );
    }
    Ok(())
}
