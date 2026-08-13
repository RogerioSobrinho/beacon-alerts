use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::Deserialize;
use thiserror::Error;

use crate::model::{AlertRecord, AlertStatus, Event, Severity};
use crate::server::{NotificationFailureUpdate, NotificationJob, ServerEventStore};

pub trait NotificationChannel: Send + Sync {
    fn name(&self) -> &str;
    fn send(&self, job: &NotificationJob) -> Result<()>;
}

#[derive(Debug, Error)]
pub enum NotificationFailure {
    #[error("temporary notification failure: {0}")]
    Temporary(String),
    #[error("permanent notification failure: {0}")]
    Permanent(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct TelegramConfig {
    pub token_file: std::path::PathBuf,
    pub chat_id: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_timeout_seconds() -> u64 {
    10
}

pub struct TelegramChannel {
    client: Client,
    token: String,
    chat_id: String,
    endpoint: String,
}

impl TelegramConfig {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let config: Self = serde_json::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("read Telegram config {}", path.display()))?,
        )
        .with_context(|| format!("parse Telegram config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.chat_id.trim().is_empty() || self.chat_id.len() > 128 {
            bail!("Telegram chat_id must be between 1 and 128 bytes");
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 120 {
            bail!("Telegram timeout_seconds must be between 1 and 120");
        }
        Ok(())
    }
}

impl TelegramChannel {
    pub fn from_config(config: TelegramConfig) -> Result<Self> {
        config.validate()?;
        let token = std::fs::read_to_string(&config.token_file)
            .with_context(|| format!("read Telegram token file {}", config.token_file.display()))?
            .trim()
            .to_owned();
        if token.is_empty() || token.len() > 512 || token.contains(char::is_whitespace) {
            bail!("Telegram token file contains an invalid token");
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .https_only(true)
            .build()
            .context("build Telegram HTTPS client")?;
        Ok(Self {
            client,
            token,
            chat_id: config.chat_id,
            endpoint: "https://api.telegram.org".into(),
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/bot{}/sendMessage", self.endpoint, self.token)
    }
}

impl NotificationChannel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn send(&self, job: &NotificationJob) -> Result<()> {
        let response = self
            .client
            .post(self.endpoint())
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "text": job.payload,
                "disable_web_page_preview": true
            }))
            .send()
            .context("send Telegram notification")?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let message = format!("Telegram returned HTTP {status}");
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(NotificationFailure::Temporary(message).into());
        }
        Err(NotificationFailure::Permanent(message).into())
    }
}

pub fn failure_is_retryable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<NotificationFailure>()
        .is_none_or(|failure| matches!(failure, NotificationFailure::Temporary(_)))
}

pub struct NotificationDispatcher {
    channels: Vec<Arc<dyn NotificationChannel>>,
    max_attempts: u32,
    retry_delay_seconds: u64,
}

impl NotificationDispatcher {
    pub fn new(
        channels: Vec<Arc<dyn NotificationChannel>>,
        max_attempts: u32,
        retry_delay_seconds: u64,
    ) -> Result<Self> {
        if channels.is_empty() {
            bail!("notification dispatcher requires at least one channel");
        }
        if max_attempts == 0 {
            bail!("notification dispatcher max_attempts must be positive");
        }
        Ok(Self {
            channels,
            max_attempts,
            retry_delay_seconds,
        })
    }

    pub fn deliver_due(&self, store: &ServerEventStore, limit: usize) -> Result<u32> {
        let mut delivered = 0;
        let now = Utc::now();
        for _ in 0..limit {
            let Some(job) = store.claim_notification(now, self.max_attempts)? else {
                break;
            };
            let Some(channel) = self
                .channels
                .iter()
                .find(|channel| channel.name() == job.channel)
            else {
                store.fail_notification(
                    job.id,
                    NotificationFailureUpdate {
                        attempts: job.attempts,
                        error: "notification channel is not configured",
                        max_attempts: self.max_attempts,
                        retry_delay_seconds: self.retry_delay_seconds,
                        retryable: false,
                        now,
                    },
                )?;
                continue;
            };
            match channel.send(&job) {
                Ok(()) => {
                    store.complete_notification(job.id, now)?;
                    delivered += 1;
                }
                Err(error) => {
                    let error_message = error.to_string();
                    store.fail_notification(
                        job.id,
                        NotificationFailureUpdate {
                            attempts: job.attempts,
                            error: &error_message,
                            max_attempts: self.max_attempts,
                            retry_delay_seconds: self.retry_delay_seconds,
                            retryable: failure_is_retryable(&error),
                            now,
                        },
                    )?
                }
            }
        }
        Ok(delivered)
    }
}

pub fn render_event(event: &Event, alert: &AlertRecord) -> String {
    let host = human_host(&event.host_id);
    let severity = human_severity(&alert.severity);
    let (icon, status, detail, action) = match (
        event.event_type.as_str(),
        &alert.status,
        event.host_id.as_str(),
    ) {
        ("storage.mount.missing", AlertStatus::Firing, "media") => (
            "🚨",
            "ALERTA ATIVO",
            "O armazenamento de mídia não está montado.",
            "Verificar o storage da Media.",
        ),
        ("storage.mount.missing", AlertStatus::Resolved, "media") => (
            "✅",
            "ALERTA RESOLVIDO",
            "O armazenamento de mídia voltou a ficar disponível.",
            "Nenhuma ação imediata; confirmar a operação normal.",
        ),
        ("backup.restic.stale", AlertStatus::Firing, _) => (
            "🚨",
            "ALERTA ATIVO",
            "A idade do backup ultrapassou o limite operacional.",
            "Verificar o último backup e o mount de destino.",
        ),
        ("backup.restic.stale", AlertStatus::Resolved, _) => (
            "✅",
            "ALERTA RESOLVIDO",
            "A idade do backup voltou ao limite operacional.",
            "Nenhuma ação imediata; confirmar o próximo ciclo.",
        ),
        (_, AlertStatus::Firing, _) => (
            "⚠️",
            "ALERTA ATIVO",
            "Uma condição monitorada requer atenção.",
            "Verificar o monitor correspondente.",
        ),
        (_, AlertStatus::Resolved, _) => (
            "✅",
            "ALERTA RESOLVIDO",
            "A condição monitorada foi normalizada.",
            "Nenhuma ação imediata.",
        ),
        (_, AlertStatus::Info, _) => (
            "ℹ️",
            "INFORMAÇÃO",
            "Um evento operacional foi registrado.",
            "Nenhuma ação imediata.",
        ),
    };

    format!(
        "{icon} {status}\n{host} | Infraestrutura\n\n{detail}\n\nAção: {action}\nSeveridade: {severity}"
    )
}

fn human_host(host: &str) -> &'static str {
    match host {
        "media" => "Media",
        "ops" => "Ops",
        "backup" => "Backup",
        "pve" => "PVE",
        "private-cloud" => "Private Cloud",
        _ => "Infraestrutura",
    }
}

fn human_severity(severity: &Severity) -> &'static str {
    match severity {
        Severity::Critical => "CRÍTICA",
        Severity::Warning => "ATENÇÃO",
        Severity::Info => "INFORMAÇÃO",
    }
}

pub(crate) fn retry_time(now: DateTime<Utc>, attempts: u32, base_seconds: u64) -> String {
    let exponent = attempts.saturating_sub(1).min(10);
    let delay = base_seconds.saturating_mul(2_u64.pow(exponent));
    (now + chrono::Duration::seconds(delay as i64)).to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventState, Severity};
    use crate::policy::{PolicyCatalog, PolicyRule};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use uuid::Uuid;

    struct FakeChannel {
        calls: AtomicU32,
        failures: u32,
    }

    impl NotificationChannel for FakeChannel {
        fn name(&self) -> &str {
            "fake"
        }

        fn send(&self, _job: &NotificationJob) -> Result<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < self.failures {
                bail!("temporary channel failure")
            }
            Ok(())
        }
    }

    fn event() -> Event {
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
            facts: BTreeMap::new(),
        }
    }

    fn policy() -> PolicyCatalog {
        PolicyCatalog {
            rules: vec![PolicyRule {
                name: "backup".into(),
                enabled: true,
                event_type: Some("backup.restic.stale".into()),
                source: None,
                host_id: None,
                state: None,
                severity: None,
                channels: vec!["fake".into()],
            }],
        }
    }

    #[test]
    fn dispatcher_retries_and_marks_job_sent() {
        let root = std::env::temp_dir().join(format!("beacon-notify-{}", Uuid::new_v4()));
        let store = ServerEventStore::open_with_policy(root.clone(), policy()).unwrap();
        store.accept(&event()).unwrap();
        let channel = Arc::new(FakeChannel {
            calls: AtomicU32::new(0),
            failures: 1,
        });
        let dispatcher = NotificationDispatcher::new(vec![channel.clone()], 3, 0).unwrap();

        assert_eq!(dispatcher.deliver_due(&store, 1).unwrap(), 0);
        assert_eq!(dispatcher.deliver_due(&store, 1).unwrap(), 1);
        assert_eq!(channel.calls.load(Ordering::SeqCst), 2);
        let jobs = store.list_notifications().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, "sent");
        assert_eq!(jobs[0].attempts, 2);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_event_does_not_duplicate_notification() {
        let root =
            std::env::temp_dir().join(format!("beacon-notify-idempotent-{}", Uuid::new_v4()));
        let store = ServerEventStore::open_with_policy(root.clone(), policy()).unwrap();
        let event = event();
        store.accept(&event).unwrap();
        store.accept(&event).unwrap();
        assert_eq!(store.list_notifications().unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_time_uses_exponential_backoff() {
        let now = Utc::now();
        let first = retry_time(now, 1, 2);
        let second = retry_time(now, 2, 2);
        assert!(second > first);
    }

    #[test]
    fn renders_storage_alert_without_internal_event_data() {
        let mut event = event();
        event.event_type = "storage.mount.missing".into();
        event.host_id = "media".into();
        event.facts = BTreeMap::from([
            ("mountpoint".into(), serde_json::json!("/private/secret")),
            ("token".into(), serde_json::json!("must-not-appear")),
        ]);
        let alert = AlertRecord {
            fingerprint: "media/storage/mount".into(),
            status: AlertStatus::Firing,
            severity: Severity::Critical,
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            host_id: event.host_id.clone(),
            opened_at: event.occurred_at.clone(),
            last_seen: event.occurred_at.clone(),
            resolved_at: None,
            event_count: 1,
            last_event_id: event.event_id.clone(),
        };

        let message = render_event(&event, &alert);
        assert_eq!(
            message,
            "🚨 ALERTA ATIVO\nMedia | Infraestrutura\n\nO armazenamento de mídia não está montado.\n\nAção: Verificar o storage da Media.\nSeveridade: CRÍTICA"
        );
        assert!(!message.contains("/private/secret"));
        assert!(!message.contains("must-not-appear"));
        assert!(!message.contains("media/storage/mount"));
    }

    #[test]
    fn renders_storage_recovery_as_normalized() {
        let mut event = event();
        event.event_type = "storage.mount.missing".into();
        event.host_id = "media".into();
        let alert = AlertRecord {
            fingerprint: "media/storage/mount".into(),
            status: AlertStatus::Resolved,
            severity: Severity::Info,
            event_type: event.event_type.clone(),
            source: event.source.clone(),
            host_id: event.host_id.clone(),
            opened_at: event.occurred_at.clone(),
            last_seen: event.occurred_at.clone(),
            resolved_at: Some(event.occurred_at.clone()),
            event_count: 2,
            last_event_id: event.event_id.clone(),
        };

        let message = render_event(&event, &alert);
        assert!(message.contains("ALERTA RESOLVIDO"));
        assert!(message.contains("voltou a ficar disponível"));
    }

    #[test]
    fn telegram_config_rejects_invalid_timeout() {
        let config = TelegramConfig {
            token_file: "token".into(),
            chat_id: "chat".into(),
            timeout_seconds: 0,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn permanent_failures_are_not_retryable() {
        let error: anyhow::Error = NotificationFailure::Permanent("bad request".into()).into();
        assert!(!failure_is_retryable(&error));
        let error: anyhow::Error = NotificationFailure::Temporary("network".into()).into();
        assert!(failure_is_retryable(&error));
    }

    #[test]
    fn telegram_channel_sends_json_to_local_test_endpoint() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 8192];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            assert!(request.contains("\"chat_id\":\"test-chat\""));
            assert!(request.contains("\"text\":\"critical firing\""));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        let channel = TelegramChannel {
            client: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            token: "test-token".into(),
            chat_id: "test-chat".into(),
            endpoint: format!("http://{address}"),
        };
        let job = NotificationJob {
            id: 1,
            event_id: "event".into(),
            fingerprint: "fingerprint".into(),
            channel: "telegram".into(),
            payload: "critical firing".into(),
            status: "in_flight".into(),
            attempts: 1,
            next_attempt_at: "2026-01-01T00:00:00Z".into(),
            last_error: None,
            retryable: true,
            sent_at: None,
        };
        channel.send(&job).unwrap();
        server.join().unwrap();
    }
}
