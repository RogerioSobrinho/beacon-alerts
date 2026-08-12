use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::Client;
use tokio::time::sleep;

use crate::model::Event;
use crate::spool::Spool;

#[derive(Debug)]
pub struct EnrollmentConfig {
    pub server: String,
    pub name: String,
    pub code_file: PathBuf,
    pub token_file: PathBuf,
    pub ca_file: Option<PathBuf>,
    pub allow_http: bool,
}

#[derive(Debug, serde::Deserialize)]
struct EnrollmentResponse {
    token: String,
}

#[derive(Debug)]
pub struct AgentConfig {
    pub server: String,
    pub spool: PathBuf,
    pub token: String,
    pub ca_file: Option<PathBuf>,
    pub allow_http: bool,
    pub limit: usize,
    pub max_attempts: usize,
    pub retry_delay_seconds: u64,
}

pub async fn drain(args: AgentConfig) -> Result<usize> {
    if args.token.trim().is_empty() {
        bail!("agent token cannot be empty");
    }
    if args.max_attempts == 0 {
        bail!("max attempts must be greater than zero");
    }
    let spool = tokio::task::spawn_blocking(move || Spool::open(args.spool))
        .await
        .context("open agent spool task")??;
    let client = build_client(&args.server, args.ca_file.as_deref(), args.allow_http)?;
    let events = tokio::task::spawn_blocking({
        let spool = spool.clone();
        move || spool.list()
    })
    .await
    .context("list agent spool task")??
    .into_iter()
    .take(args.limit)
    .collect::<Vec<_>>();
    let mut delivered = 0;
    for event in events {
        deliver_with_retry(
            &client,
            &args.server,
            &args.token,
            &event,
            args.max_attempts,
            args.retry_delay_seconds,
        )
        .await?;
        let remove_spool = spool.clone();
        tokio::task::spawn_blocking(move || remove_spool.remove(&event))
            .await
            .context("remove delivered event task")??;
        delivered += 1;
    }
    Ok(delivered)
}

pub async fn enroll(args: EnrollmentConfig) -> Result<()> {
    if args.name.trim().is_empty() {
        bail!("agent name cannot be empty");
    }
    let code = fs::read_to_string(&args.code_file)
        .with_context(|| format!("read enrollment code file {}", args.code_file.display()))?;
    let code = code.trim();
    if code.is_empty() {
        bail!("enrollment code file {} is empty", args.code_file.display());
    }
    let client = build_client(&args.server, args.ca_file.as_deref(), args.allow_http)?;
    let url = format!("{}/v1/enroll", args.server.trim_end_matches('/'));
    let response = client
        .post(url)
        .json(&serde_json::json!({ "name": args.name, "code": code }))
        .send()
        .await
        .context("send enrollment request to server")?;
    if !response.status().is_success() {
        bail!("server returned {}", response.status());
    }
    let enrollment: EnrollmentResponse = response
        .json()
        .await
        .context("decode enrollment response")?;
    if enrollment.token.trim().is_empty() {
        bail!("server returned an empty agent token");
    }
    write_token(&args.token_file, &enrollment.token)
}

fn write_token(path: &Path, token: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("agent token file must have a parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create agent token directory {}", parent.display()))?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .with_context(|| format!("create temporary agent token file {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(token.trim().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        let _ = fs::remove_file(&temporary);
        bail!(
            "agent token file {} already exists; remove it before enrolling",
            path.display()
        );
    }
    fs::hard_link(&temporary, path).with_context(|| {
        format!(
            "install agent token file {}; it may already exist",
            path.display()
        )
    })?;
    fs::remove_file(temporary)?;
    Ok(())
}

fn build_client(
    server: &str,
    ca_file: Option<&std::path::Path>,
    allow_http: bool,
) -> Result<Client> {
    let parsed = reqwest::Url::parse(server).context("parse Beacon server URL")?;
    if parsed.scheme() != "https" && !allow_http {
        bail!("Beacon agent requires an HTTPS server URL; use --allow-http only for local development");
    }
    let mut builder = Client::builder().timeout(Duration::from_secs(10));
    if let Some(path) = ca_file {
        let pem = std::fs::read(path)
            .with_context(|| format!("read Beacon CA certificate {}", path.display()))?;
        let certificate = reqwest::Certificate::from_pem(&pem)
            .with_context(|| format!("parse Beacon CA certificate {}", path.display()))?;
        builder = builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate);
    }
    Ok(builder.build()?)
}

async fn deliver_with_retry(
    client: &Client,
    server: &str,
    token: &str,
    event: &Event,
    max_attempts: usize,
    retry_delay_seconds: u64,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match submit_event(client, server, token, event).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < max_attempts {
                    sleep(Duration::from_secs(retry_delay_seconds)).await;
                }
            }
        }
    }
    Err(last_error.expect("max_attempts guarantees one attempt"))
}

async fn submit_event(client: &Client, server: &str, token: &str, event: &Event) -> Result<()> {
    let url = format!("{}/v1/events", server.trim_end_matches('/'));
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(event)
        .send()
        .await
        .context("send event to server")?;
    if !response.status().is_success() {
        bail!("server returned {}", response.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "server")]
    use crate::model::{EventState, Severity};
    #[cfg(feature = "server")]
    use crate::server::{router, serve_listener};
    #[cfg(feature = "server")]
    use std::collections::BTreeMap;
    #[cfg(feature = "server")]
    use tokio::net::TcpListener;
    #[cfg(feature = "server")]
    use uuid::Uuid;

    #[test]
    fn agent_rejects_http_without_explicit_development_flag() {
        let error = build_client("http://127.0.0.1:8787", None, false).unwrap_err();
        assert!(error.to_string().contains("requires an HTTPS"));
    }

    #[cfg(feature = "server")]
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

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn drain_delivers_event_and_removes_it_after_ack() {
        let server_root =
            std::env::temp_dir().join(format!("beacon-agent-server-{}", Uuid::new_v4()));
        let spool_root =
            std::env::temp_dir().join(format!("beacon-agent-spool-{}", Uuid::new_v4()));
        let spool = Spool::open(spool_root.clone()).unwrap();
        let event = test_event();
        spool.enqueue(&event).unwrap();
        drop(spool);

        let credentials =
            std::env::temp_dir().join(format!("beacon-agent-credentials-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&credentials).unwrap();
        std::fs::write(credentials.join("backup.token"), "test-token\n").unwrap();
        let app = router(server_root.clone(), credentials.clone()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_task = tokio::spawn(serve_listener(listener, app));

        let delivered = drain(AgentConfig {
            server: format!("http://{address}"),
            spool: spool_root.clone(),
            token: "test-token".into(),
            ca_file: None,
            allow_http: true,
            limit: 10,
            max_attempts: 1,
            retry_delay_seconds: 0,
        })
        .await
        .unwrap();

        assert_eq!(delivered, 1);
        assert!(Spool::open(spool_root.clone())
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
        assert!(server_root.join("beacon.sqlite3").exists());

        server_task.abort();
        let _ = server_task.await;
        std::fs::remove_dir_all(credentials).unwrap();
        std::fs::remove_dir_all(server_root).unwrap();
        std::fs::remove_dir_all(spool_root).unwrap();
    }
}
