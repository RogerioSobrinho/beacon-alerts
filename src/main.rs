use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
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

impl EventState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
            Self::Info => "info",
        }
    }
}

impl Severity {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl Event {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported event schema version: {}", self.schema_version);
        }
        require_field("event_id", &self.event_id, 128)?;
        Uuid::parse_str(&self.event_id).context("event_id must be a UUID")?;
        require_field("event_type", &self.event_type, 128)?;
        require_field("source", &self.source, 128)?;
        require_field("host_id", &self.host_id, 128)?;
        require_field("fingerprint", &self.fingerprint, 256)?;
        require_field("occurred_at", &self.occurred_at, 64)?;
        DateTime::parse_from_rfc3339(&self.occurred_at).context("occurred_at must be RFC3339")?;
        if self.facts.len() > 32 {
            bail!("facts cannot contain more than 32 fields");
        }
        for (key, value) in &self.facts {
            require_field("fact key", key, 64)?;
            if serde_json::to_vec(value)?.len() > 4096 {
                bail!("fact '{key}' exceeds the 4096-byte value limit");
            }
        }
        Ok(())
    }
}

fn require_field(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} cannot be empty");
    }
    if value.len() > max_bytes {
        bail!("{name} exceeds the {max_bytes}-byte limit");
    }
    Ok(())
}

struct Spool {
    pending: PathBuf,
}

impl Spool {
    fn open(root: PathBuf) -> Result<Self> {
        let pending = root.join("pending");
        fs::create_dir_all(&pending)
            .with_context(|| format!("create spool directory {}", pending.display()))?;
        Ok(Self { pending })
    }

    fn enqueue(&self, event: &Event) -> Result<PathBuf> {
        event.validate()?;
        let filename = format!("{}.json", event.event_id);
        let destination = self.pending.join(filename);
        if destination.exists() {
            bail!("event {} is already queued", event.event_id);
        }
        let temporary = self.pending.join(format!(".{}.tmp", event.event_id));
        let encoded = serde_json::to_vec_pretty(event)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .with_context(|| format!("create temporary spool file {}", temporary.display()))?;
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &destination)
            .with_context(|| format!("commit event {} to spool", event.event_id))?;
        Ok(destination)
    }

    fn list(&self) -> Result<Vec<Event>> {
        let mut paths = fs::read_dir(&self.pending)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        paths
            .into_iter()
            .map(|path| {
                let event: Event = serde_json::from_reader(
                    File::open(&path).with_context(|| format!("open {}", path.display()))?,
                )
                .with_context(|| format!("decode {}", path.display()))?;
                event.validate()?;
                Ok(event)
            })
            .collect()
    }
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
        Command::Send(args) => send_event(args)?,
        Command::Replay(args) => replay_events(args)?,
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_event() -> Event {
        Event {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            event_type: "backup.restic.stale".into(),
            source: "test".into(),
            host_id: "backup".into(),
            state: EventState::Firing,
            severity: Severity::Critical,
            fingerprint: "backup/restic/age".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            facts: BTreeMap::from([("age_hours".into(), serde_json::json!(41))]),
        }
    }

    #[test]
    fn event_round_trips_and_validates() {
        let event = test_event();
        let encoded = serde_json::to_string(&event).unwrap();
        let decoded: Event = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
    }

    #[test]
    fn spool_writes_and_lists_events() {
        let root = std::env::temp_dir().join(format!("beacon-test-{}", Uuid::new_v4()));
        let spool = Spool::open(root.clone()).unwrap();
        let event = test_event();
        let path = spool.enqueue(&event).unwrap();
        assert!(path.exists());
        assert_eq!(spool.list().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_schema_is_rejected() {
        let mut event = test_event();
        event.schema_version = 2;
        assert!(event.validate().is_err());
    }

    #[test]
    fn invalid_identity_and_timestamp_are_rejected() {
        let mut event = test_event();
        event.event_id = "../../outside-spool".into();
        assert!(event.validate().is_err());

        let mut event = test_event();
        event.occurred_at = "not-a-timestamp".into();
        assert!(event.validate().is_err());
    }
}
