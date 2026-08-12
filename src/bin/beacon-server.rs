#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use beacon_alerts::policy::PolicyCatalog;
use beacon_alerts::server::{create_enrollment, serve, ServerConfig};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "beacon-server", version, about = "Beacon central server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the authenticated Beacon server.
    Run(ServerArgs),
    /// Create a one-time enrollment code for an agent.
    Agent(AgentCommand),
}

#[derive(Debug, Args)]
struct AgentCommand {
    #[command(subcommand)]
    command: AgentSubcommand,
}

#[derive(Debug, Subcommand)]
enum AgentSubcommand {
    Create(CreateAgentArgs),
}

#[derive(Debug, Args)]
struct CreateAgentArgs {
    #[arg(long)]
    name: String,
    #[arg(long, default_value = "/var/lib/beacon/events")]
    data: PathBuf,
    #[arg(long, default_value = "/etc/beacon/agents.d")]
    credentials_dir: PathBuf,
    /// Write the one-time code to this restricted file.
    #[arg(long)]
    code_file: PathBuf,
    #[arg(long, default_value_t = 900)]
    ttl_seconds: u64,
}

#[derive(Debug, Args)]
struct ServerArgs {
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,
    #[arg(long, default_value = "/var/lib/beacon/events")]
    data: PathBuf,
    #[arg(long, default_value = "/etc/beacon/agents.d")]
    credentials_dir: PathBuf,
    #[arg(long)]
    policy_file: Option<PathBuf>,
    #[arg(long)]
    tls_cert: Option<PathBuf>,
    #[arg(long)]
    tls_key: Option<PathBuf>,
    #[arg(long)]
    allow_http: bool,
    #[arg(long)]
    telegram_config: Option<PathBuf>,
    #[arg(long, default_value_t = 30)]
    notify_interval_seconds: u64,
    #[arg(long, default_value_t = 100)]
    notify_limit: usize,
    #[arg(long, default_value_t = 3)]
    notify_max_attempts: u32,
    #[arg(long, default_value_t = 30)]
    notify_retry_delay_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run(args) => {
            serve(ServerConfig {
                bind: args.bind,
                data: args.data,
                credentials_dir: args.credentials_dir,
                policy: read_policy(args.policy_file.as_deref())?,
                tls_cert: args.tls_cert,
                tls_key: args.tls_key,
                allow_http: args.allow_http,
                telegram_config: args.telegram_config,
                notify_interval_seconds: args.notify_interval_seconds,
                notify_limit: args.notify_limit,
                notify_max_attempts: args.notify_max_attempts,
                notify_retry_delay_seconds: args.notify_retry_delay_seconds,
            })
            .await?;
        }
        Command::Agent(AgentCommand {
            command: AgentSubcommand::Create(args),
        }) => {
            create_enrollment(
                args.data,
                args.credentials_dir,
                args.name,
                args.code_file.clone(),
                args.ttl_seconds,
            )?;
            println!("enrollment code written to {}", args.code_file.display());
        }
    }
    Ok(())
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
