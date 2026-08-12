use std::sync::Arc;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};

use crate::model::{AlertRecord, Event};
use crate::server::{NotificationJob, ServerEventStore};

pub trait NotificationChannel: Send + Sync {
    fn name(&self) -> &str;
    fn send(&self, job: &NotificationJob) -> Result<()>;
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
                    job.attempts,
                    "notification channel is not configured",
                    self.max_attempts,
                    self.retry_delay_seconds,
                    now,
                )?;
                continue;
            };
            match channel.send(&job) {
                Ok(()) => {
                    store.complete_notification(job.id, now)?;
                    delivered += 1;
                }
                Err(error) => store.fail_notification(
                    job.id,
                    job.attempts,
                    &error.to_string(),
                    self.max_attempts,
                    self.retry_delay_seconds,
                    now,
                )?,
            }
        }
        Ok(delivered)
    }
}

pub fn render_event(event: &Event, alert: &AlertRecord) -> String {
    format!(
        "{} {} on {}: {} ({})",
        alert.severity.as_str(),
        alert.status.as_str(),
        event.host_id,
        event.event_type,
        event.fingerprint
    )
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
}
