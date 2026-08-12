use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::Client;
use tokio::time::sleep;

use crate::model::Event;
use crate::spool::Spool;

#[derive(Debug)]
pub struct AgentConfig {
    pub server: String,
    pub spool: PathBuf,
    pub token: String,
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
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
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
    use crate::model::{EventState, Severity};
    use crate::server::{router, serve_listener};
    use std::collections::BTreeMap;
    use tokio::net::TcpListener;
    use uuid::Uuid;

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

        let app = router(server_root.clone(), "test-token".into()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_task = tokio::spawn(serve_listener(listener, app));

        let delivered = drain(AgentConfig {
            server: format!("http://{address}"),
            spool: spool_root.clone(),
            token: "test-token".into(),
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
        std::fs::remove_dir_all(server_root).unwrap();
        std::fs::remove_dir_all(spool_root).unwrap();
    }
}
