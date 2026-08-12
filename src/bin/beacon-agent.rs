#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use beacon_alerts::agent::{drain, enroll, AgentConfig, EnrollmentConfig};
use beacon_alerts::model::{Event, EventState, Severity};
use beacon_alerts::spool::Spool;
use chrono::{SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "beacon-agent", version, about = "Beacon producer-host client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Enroll this host and receive a server-generated agent token.
    Enroll(EnrollArgs),
    /// Deliver queued events to the Beacon server.
    Run(RunArgs),
    /// Create and queue a normalized event.
    Send(SendArgs),
    /// Inspect events waiting in the local spool.
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
struct EnrollArgs {
    #[arg(long)]
    server: String,
    #[arg(long)]
    name: String,
    /// One-time code file created by beacon-server agent create.
    #[arg(long)]
    code_file: PathBuf,
    #[arg(long, default_value = "/etc/beacon/agent.token")]
    token_file: PathBuf,
    #[arg(long)]
    allow_http: bool,
    #[arg(long)]
    ca_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long, default_value = "https://127.0.0.1:8787")]
    server: String,
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
    #[arg(long, default_value = "/etc/beacon/agent.token")]
    token_file: PathBuf,
    #[arg(long)]
    ca_file: Option<PathBuf>,
    #[arg(long)]
    allow_http: bool,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long, default_value_t = 3)]
    max_attempts: usize,
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
    #[arg(long, default_value = "{}")]
    facts: String,
    #[arg(long, default_value = "/var/lib/beacon/spool")]
    spool: PathBuf,
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
        Command::Enroll(args) => {
            enroll(EnrollmentConfig {
                server: args.server,
                name: args.name,
                code_file: args.code_file,
                token_file: args.token_file,
                ca_file: args.ca_file,
                allow_http: args.allow_http,
            })
            .await?;
            println!("agent enrolled; token stored in the configured token file");
        }
        Command::Run(args) => {
            let delivered = drain(AgentConfig {
                server: args.server,
                spool: args.spool,
                token: read_token(&args.token_file)?,
                ca_file: args.ca_file,
                allow_http: args.allow_http,
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
    let token = fs::read_to_string(path)
        .with_context(|| format!("read agent token file {}", path.display()))?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("agent token file {} is empty", path.display());
    }
    Ok(token)
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
    let events = Spool::open(args.spool)?.list()?;
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
