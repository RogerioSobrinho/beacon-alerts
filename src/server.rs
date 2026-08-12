use std::fs::{self, OpenOptions};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::net::TcpListener;

use crate::model::{AlertRecord, AlertStatus, Event, EventState};
use crate::notification::{
    render_event, retry_time, NotificationChannel, NotificationDispatcher, TelegramChannel,
    TelegramConfig,
};
use crate::policy::PolicyCatalog;

#[derive(Debug)]
pub struct ServerConfig {
    pub bind: String,
    pub data: PathBuf,
    pub credentials_dir: PathBuf,
    pub policy: PolicyCatalog,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    pub allow_http: bool,
    pub telegram_config: Option<PathBuf>,
    pub notify_interval_seconds: u64,
    pub notify_limit: usize,
    pub notify_max_attempts: u32,
    pub notify_retry_delay_seconds: u64,
}

#[derive(Clone)]
pub struct ServerState {
    credentials: Arc<CredentialStore>,
    events: Arc<ServerEventStore>,
}

#[derive(Clone)]
pub struct CredentialStore {
    directory: PathBuf,
}

impl CredentialStore {
    pub fn new(directory: PathBuf) -> Result<Self> {
        if !directory.is_dir() {
            bail!(
                "agent credential directory does not exist or is not a directory: {}",
                directory.display()
            );
        }
        Ok(Self { directory })
    }

    fn authorized(&self, presented: &str) -> Result<bool> {
        let entries = fs::read_dir(&self.directory).with_context(|| {
            format!(
                "read agent credential directory {}",
                self.directory.display()
            )
        })?;
        let mut matched = false;
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_file() {
                continue;
            }
            let token = fs::read_to_string(entry.path())?.trim().to_owned();
            if token.is_empty() {
                continue;
            }
            let equal: bool = token.as_bytes().ct_eq(presented.as_bytes()).into();
            matched |= equal;
        }
        Ok(matched)
    }
}

pub struct ServerEventStore {
    database: PathBuf,
    policy: PolicyCatalog,
    _lock: std::fs::File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationJob {
    pub id: i64,
    pub event_id: String,
    pub fingerprint: String,
    pub channel: String,
    pub payload: String,
    pub status: String,
    pub attempts: u32,
    pub next_attempt_at: String,
    pub last_error: Option<String>,
    pub retryable: bool,
    pub sent_at: Option<String>,
}

pub struct NotificationFailureUpdate<'a> {
    pub attempts: u32,
    pub error: &'a str,
    pub max_attempts: u32,
    pub retry_delay_seconds: u64,
    pub retryable: bool,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Error)]
#[error("event_id already exists with a different payload")]
struct EventConflict;

#[derive(Debug, Error)]
#[error("event_id was accepted but its alert state is missing")]
struct MissingAlertState;

#[derive(Debug)]
pub(crate) struct EventResult {
    duplicate: bool,
    alert: AlertRecord,
}

#[derive(Debug, Deserialize)]
struct AlertQuery {
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct EnrollmentRequest {
    name: String,
    code: String,
}

fn default_limit() -> usize {
    100
}

impl ServerEventStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        Self::open_with_policy(root, PolicyCatalog::default())
    }

    pub fn open_with_policy(root: PathBuf, policy: PolicyCatalog) -> Result<Self> {
        policy.validate()?;
        fs::create_dir_all(&root)
            .with_context(|| format!("create server data directory {}", root.display()))?;
        let lock_path = root.join(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("open server data lock {}", lock_path.display()))?;
        lock.try_lock_exclusive().with_context(|| {
            format!(
                "lock server data {}; another Beacon server may be using it",
                root.display()
            )
        })?;

        let database = root.join("beacon.sqlite3");
        let connection = Connection::open(&database)
            .with_context(|| format!("open server database {}", database.display()))?;
        initialize_database(&connection)?;
        drop(connection);

        Ok(Self {
            database,
            policy,
            _lock: lock,
        })
    }

    #[cfg(test)]
    fn create_enrollment(&self, name: &str, code: &str, ttl_seconds: u64) -> Result<()> {
        validate_agent_name(name)?;
        if code.trim().is_empty() {
            bail!("enrollment code cannot be empty");
        }
        if ttl_seconds == 0 {
            bail!("enrollment TTL must be greater than zero");
        }
        let connection = Connection::open(&self.database)?;
        connection.execute(
            "INSERT INTO enrollments (code_hash, name, expires_at) VALUES (?1, ?2, ?3)",
            params![
                hash_secret(code),
                name,
                (Utc::now() + chrono::Duration::seconds(ttl_seconds as i64)).to_rfc3339()
            ],
        )?;
        Ok(())
    }

    fn consume_enrollment(
        &self,
        name: &str,
        code: &str,
        credentials_dir: &std::path::Path,
    ) -> Result<String> {
        validate_agent_name(name)?;
        if code.trim().is_empty() {
            bail!("enrollment code cannot be empty");
        }
        let credential_path = credentials_dir.join(format!("{name}.token"));
        if credential_path.exists() {
            bail!("agent credential already exists for {name}");
        }
        let connection = Connection::open(&self.database)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let transaction = connection.unchecked_transaction()?;
        let Some((enrolled_name, expires_at, consumed_at)) = transaction
            .query_row(
                "SELECT name, expires_at, consumed_at FROM enrollments WHERE code_hash = ?1",
                params![hash_secret(code)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            bail!("invalid enrollment code");
        };
        if enrolled_name != name {
            bail!("enrollment code does not match agent name");
        }
        if consumed_at.is_some() {
            bail!("enrollment code has already been used");
        }
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
            .context("parse enrollment expiry")?
            .with_timezone(&Utc);
        if Utc::now() >= expires_at {
            bail!("enrollment code has expired");
        }
        let token = uuid::Uuid::new_v4().to_string();
        transaction.execute(
            "UPDATE enrollments SET consumed_at = ?1 WHERE code_hash = ?2 AND consumed_at IS NULL",
            params![Utc::now().to_rfc3339(), hash_secret(code)],
        )?;
        write_credential(&credential_path, &token)?;
        transaction.commit()?;
        Ok(token)
    }

    pub(crate) fn accept(&self, event: &Event) -> Result<EventResult> {
        event.validate()?;
        let connection = Connection::open(&self.database)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let transaction = connection.unchecked_transaction()?;

        let payload = serde_json::to_string(event)?;
        if let Some(existing_payload) = transaction
            .query_row(
                "SELECT payload FROM events WHERE event_id = ?1",
                params![event.event_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing_payload != payload {
                return Err(EventConflict.into());
            }
            let alert = load_alert(&transaction, &event.fingerprint)?
                .ok_or_else(|| anyhow::Error::new(MissingAlertState))?;
            return Ok(EventResult {
                duplicate: true,
                alert,
            });
        }

        transaction.execute(
            "INSERT INTO events (event_id, payload, fingerprint, received_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                event.event_id,
                payload,
                event.fingerprint,
                Utc::now().to_rfc3339()
            ],
        )?;
        let alert = transition_alert(&transaction, event)?;
        if event.state != EventState::Firing || alert.event_count == 1 {
            enqueue_notifications(
                &transaction,
                event,
                &alert,
                &self.policy.channels_for(event),
            )?;
        }
        transaction.commit()?;

        Ok(EventResult {
            duplicate: false,
            alert,
        })
    }

    fn list_alerts(&self, status: Option<AlertStatus>, limit: usize) -> Result<Vec<AlertRecord>> {
        let connection = Connection::open(&self.database)?;
        let limit = limit.clamp(1, 500) as i64;
        let mut alerts = Vec::new();
        if let Some(status) = status {
            let mut statement = connection.prepare(
                "SELECT fingerprint, status, severity, event_type, source, host_id,
                        opened_at, last_seen, resolved_at, event_count, last_event_id
                 FROM alerts WHERE status = ?1 ORDER BY last_seen DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![status.as_str(), limit], alert_from_row)?;
            for row in rows {
                alerts.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT fingerprint, status, severity, event_type, source, host_id,
                        opened_at, last_seen, resolved_at, event_count, last_event_id
                 FROM alerts ORDER BY last_seen DESC LIMIT ?1",
            )?;
            let rows = statement.query_map(params![limit], alert_from_row)?;
            for row in rows {
                alerts.push(row?);
            }
        }
        Ok(alerts)
    }

    pub fn claim_notification(
        &self,
        now: DateTime<Utc>,
        max_attempts: u32,
    ) -> Result<Option<NotificationJob>> {
        let connection = Connection::open(&self.database)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let transaction = connection.unchecked_transaction()?;
        let now_string = now.to_rfc3339();
        let stale_before = (now - chrono::Duration::minutes(5)).to_rfc3339();
        let job = transaction
            .query_row(
                "SELECT id, event_id, fingerprint, channel, payload, status, attempts,
                        next_attempt_at, last_error, retryable, sent_at
                 FROM notifications
                 WHERE attempts < ?1 AND (
                     status = 'pending'
                     OR (status = 'failed' AND retryable = 1 AND next_attempt_at <= ?2)
                     OR (status = 'in_flight' AND claimed_at <= ?3)
                 )
                 ORDER BY id LIMIT 1",
                params![max_attempts, now_string, stale_before],
                notification_from_row,
            )
            .optional()?;
        let Some(job) = job else {
            return Ok(None);
        };
        transaction.execute(
            "UPDATE notifications
             SET status = 'in_flight', attempts = attempts + 1, claimed_at = ?1
             WHERE id = ?2",
            params![now.to_rfc3339(), job.id],
        )?;
        transaction.commit()?;
        Ok(Some(NotificationJob {
            attempts: job.attempts + 1,
            status: "in_flight".into(),
            ..job
        }))
    }

    pub fn complete_notification(&self, id: i64, now: DateTime<Utc>) -> Result<()> {
        let connection = Connection::open(&self.database)?;
        connection.execute(
            "UPDATE notifications
             SET status = 'sent', sent_at = ?1, claimed_at = NULL, last_error = NULL
             WHERE id = ?2 AND status = 'in_flight'",
            params![now.to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn fail_notification(&self, id: i64, update: NotificationFailureUpdate<'_>) -> Result<()> {
        let connection = Connection::open(&self.database)?;
        let status = "failed";
        let next_attempt_at = if !update.retryable || update.attempts >= update.max_attempts {
            update.now.to_rfc3339()
        } else {
            retry_time(update.now, update.attempts, update.retry_delay_seconds)
        };
        connection.execute(
            "UPDATE notifications
             SET status = ?1, next_attempt_at = ?2, last_error = ?3,
                 retryable = ?4, claimed_at = NULL
             WHERE id = ?5 AND status = 'in_flight'",
            params![status, next_attempt_at, update.error, update.retryable, id],
        )?;
        Ok(())
    }

    pub fn list_notifications(&self) -> Result<Vec<NotificationJob>> {
        let connection = Connection::open(&self.database)?;
        let mut statement = connection.prepare(
            "SELECT id, event_id, fingerprint, channel, payload, status, attempts,
                    next_attempt_at, last_error, retryable, sent_at
             FROM notifications ORDER BY id",
        )?;
        let rows = statement.query_map([], notification_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn initialize_database(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS events (
             event_id TEXT PRIMARY KEY,
             payload TEXT NOT NULL,
             fingerprint TEXT NOT NULL,
             received_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS alerts (
             fingerprint TEXT PRIMARY KEY,
             status TEXT NOT NULL,
             severity TEXT NOT NULL,
             event_type TEXT NOT NULL,
             source TEXT NOT NULL,
             host_id TEXT NOT NULL,
             opened_at TEXT NOT NULL,
             last_seen TEXT NOT NULL,
             resolved_at TEXT,
             event_count INTEGER NOT NULL,
             last_event_id TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS notifications (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id TEXT NOT NULL,
             fingerprint TEXT NOT NULL,
             channel TEXT NOT NULL,
             payload TEXT NOT NULL,
             status TEXT NOT NULL,
             attempts INTEGER NOT NULL DEFAULT 0,
             next_attempt_at TEXT NOT NULL,
             last_error TEXT,
             retryable INTEGER NOT NULL DEFAULT 1,
             claimed_at TEXT,
             sent_at TEXT,
             UNIQUE(event_id, channel)
         );
         CREATE TABLE IF NOT EXISTS enrollments (
             code_hash TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             expires_at TEXT NOT NULL,
             consumed_at TEXT
         );",
    )?;
    let has_retryable = connection
        .prepare("PRAGMA table_info(notifications)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .any(|name| name == "retryable");
    if !has_retryable {
        connection.execute(
            "ALTER TABLE notifications ADD COLUMN retryable INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    Ok(())
}

fn enqueue_notifications(
    transaction: &Transaction<'_>,
    event: &Event,
    alert: &AlertRecord,
    channels: &[String],
) -> Result<()> {
    let payload = render_event(event, alert);
    let next_attempt_at = Utc::now().to_rfc3339();
    for channel in channels {
        transaction.execute(
            "INSERT OR IGNORE INTO notifications
             (event_id, fingerprint, channel, payload, status, attempts, next_attempt_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5)",
            params![
                event.event_id,
                event.fingerprint,
                channel,
                payload,
                next_attempt_at
            ],
        )?;
    }
    Ok(())
}

fn transition_alert(transaction: &Transaction<'_>, event: &Event) -> Result<AlertRecord> {
    let existing = load_alert(transaction, &event.fingerprint)?;
    let alert = match event.state {
        EventState::Firing => match existing {
            Some(mut alert) if alert.status == AlertStatus::Firing => {
                alert.severity = event.severity.clone();
                alert.event_type = event.event_type.clone();
                alert.source = event.source.clone();
                alert.host_id = event.host_id.clone();
                alert.last_seen = event.occurred_at.clone();
                alert.resolved_at = None;
                alert.event_count += 1;
                alert.last_event_id = event.event_id.clone();
                alert
            }
            Some(mut alert) => {
                alert.status = AlertStatus::Firing;
                alert.severity = event.severity.clone();
                alert.event_type = event.event_type.clone();
                alert.source = event.source.clone();
                alert.host_id = event.host_id.clone();
                alert.opened_at = event.occurred_at.clone();
                alert.last_seen = event.occurred_at.clone();
                alert.resolved_at = None;
                alert.event_count = 1;
                alert.last_event_id = event.event_id.clone();
                alert
            }
            None => new_alert(event, AlertStatus::Firing),
        },
        EventState::Resolved => match existing {
            Some(mut alert) => {
                alert.status = AlertStatus::Resolved;
                alert.severity = event.severity.clone();
                alert.last_seen = event.occurred_at.clone();
                alert.resolved_at = Some(event.occurred_at.clone());
                alert.event_count += 1;
                alert.last_event_id = event.event_id.clone();
                alert
            }
            None => new_alert(event, AlertStatus::Resolved),
        },
        EventState::Info => match existing {
            Some(mut alert) if alert.status == AlertStatus::Firing => {
                alert.last_seen = event.occurred_at.clone();
                alert.event_count += 1;
                alert.last_event_id = event.event_id.clone();
                alert
            }
            Some(mut alert) => {
                alert.status = AlertStatus::Info;
                alert.severity = event.severity.clone();
                alert.event_type = event.event_type.clone();
                alert.source = event.source.clone();
                alert.host_id = event.host_id.clone();
                alert.last_seen = event.occurred_at.clone();
                alert.resolved_at = None;
                alert.event_count += 1;
                alert.last_event_id = event.event_id.clone();
                alert
            }
            None => new_alert(event, AlertStatus::Info),
        },
    };
    save_alert(transaction, &alert)?;
    Ok(alert)
}

fn new_alert(event: &Event, status: AlertStatus) -> AlertRecord {
    AlertRecord {
        fingerprint: event.fingerprint.clone(),
        status,
        severity: event.severity.clone(),
        event_type: event.event_type.clone(),
        source: event.source.clone(),
        host_id: event.host_id.clone(),
        opened_at: event.occurred_at.clone(),
        last_seen: event.occurred_at.clone(),
        resolved_at: (event.state == EventState::Resolved).then(|| event.occurred_at.clone()),
        event_count: 1,
        last_event_id: event.event_id.clone(),
    }
}

fn save_alert(transaction: &Transaction<'_>, alert: &AlertRecord) -> Result<()> {
    transaction.execute(
        "INSERT INTO alerts (
             fingerprint, status, severity, event_type, source, host_id,
             opened_at, last_seen, resolved_at, event_count, last_event_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(fingerprint) DO UPDATE SET
             status = excluded.status,
             severity = excluded.severity,
             event_type = excluded.event_type,
             source = excluded.source,
             host_id = excluded.host_id,
             opened_at = excluded.opened_at,
             last_seen = excluded.last_seen,
             resolved_at = excluded.resolved_at,
             event_count = excluded.event_count,
             last_event_id = excluded.last_event_id",
        params![
            alert.fingerprint,
            alert.status.as_str(),
            alert.severity.as_str(),
            alert.event_type,
            alert.source,
            alert.host_id,
            alert.opened_at,
            alert.last_seen,
            alert.resolved_at,
            alert.event_count,
            alert.last_event_id,
        ],
    )?;
    Ok(())
}

fn load_alert(transaction: &Transaction<'_>, fingerprint: &str) -> Result<Option<AlertRecord>> {
    transaction
        .query_row(
            "SELECT fingerprint, status, severity, event_type, source, host_id,
                    opened_at, last_seen, resolved_at, event_count, last_event_id
             FROM alerts WHERE fingerprint = ?1",
            params![fingerprint],
            alert_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn alert_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlertRecord> {
    let status: String = row.get(1)?;
    let severity: String = row.get(2)?;
    Ok(AlertRecord {
        fingerprint: row.get(0)?,
        status: status.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        severity: severity
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        event_type: row.get(3)?,
        source: row.get(4)?,
        host_id: row.get(5)?,
        opened_at: row.get(6)?,
        last_seen: row.get(7)?,
        resolved_at: row.get(8)?,
        event_count: row.get(9)?,
        last_event_id: row.get(10)?,
    })
}

fn notification_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NotificationJob> {
    Ok(NotificationJob {
        id: row.get(0)?,
        event_id: row.get(1)?,
        fingerprint: row.get(2)?,
        channel: row.get(3)?,
        payload: row.get(4)?,
        status: row.get(5)?,
        attempts: row.get(6)?,
        next_attempt_at: row.get(7)?,
        last_error: row.get(8)?,
        retryable: row.get::<_, i64>(9)? != 0,
        sent_at: row.get(10)?,
    })
}

pub async fn serve(args: ServerConfig) -> Result<()> {
    let bind: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid server bind address {}", args.bind))?;
    let state = server_state(args.data, args.credentials_dir, args.policy)?;
    if let Some(config_path) = args.telegram_config {
        let channel: Arc<dyn NotificationChannel> = Arc::new(TelegramChannel::from_config(
            TelegramConfig::load(&config_path)?,
        )?);
        let dispatcher = Arc::new(NotificationDispatcher::new(
            vec![channel],
            args.notify_max_attempts,
            args.notify_retry_delay_seconds,
        )?);
        spawn_notification_worker(
            state.events.clone(),
            dispatcher,
            args.notify_interval_seconds,
            args.notify_limit,
        )?;
    }
    let router = build_router(state);
    match (args.tls_cert, args.tls_key) {
        (Some(cert), Some(key)) => {
            let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            println!("Beacon server listening with TLS on {bind}");
            axum_server::bind_rustls(bind, config)
                .serve(router.into_make_service())
                .await?;
        }
        (None, None) => {
            if !args.allow_http {
                bail!("Beacon server requires TLS; use --allow-http only for local development");
            }
            let listener = TcpListener::bind(bind).await?;
            println!(
                "Beacon server listening without TLS on {}",
                listener.local_addr()?
            );
            axum::serve(listener, router).await?;
        }
        _ => bail!("tls_cert and tls_key must be configured together"),
    }
    Ok(())
}

pub async fn serve_listener(listener: TcpListener, router: Router) -> Result<()> {
    axum::serve(listener, router).await?;
    Ok(())
}

pub fn router(data: PathBuf, credentials_dir: PathBuf) -> Result<Router> {
    router_with_credentials(data, credentials_dir, PolicyCatalog::default())
}

pub fn router_with_credentials(
    data: PathBuf,
    credentials_dir: PathBuf,
    policy: PolicyCatalog,
) -> Result<Router> {
    Ok(build_router(server_state(data, credentials_dir, policy)?))
}

pub fn create_enrollment(
    data: PathBuf,
    credentials_dir: PathBuf,
    name: String,
    code_file: PathBuf,
    ttl_seconds: u64,
) -> Result<()> {
    fs::create_dir_all(&credentials_dir).with_context(|| {
        format!(
            "create agent credential directory {}",
            credentials_dir.display()
        )
    })?;
    let code = uuid::Uuid::new_v4().to_string();
    validate_agent_name(&name)?;
    if ttl_seconds == 0 {
        bail!("enrollment TTL must be greater than zero");
    }
    write_secret_file(&code_file, &code)?;
    let result = (|| {
        fs::create_dir_all(&data)?;
        let connection = Connection::open(data.join("beacon.sqlite3"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        initialize_database(&connection)?;
        connection.execute(
            "INSERT INTO enrollments (code_hash, name, expires_at) VALUES (?1, ?2, ?3)",
            params![
                hash_secret(&code),
                name,
                (Utc::now() + chrono::Duration::seconds(ttl_seconds as i64)).to_rfc3339()
            ],
        )?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&code_file);
    }
    result
}

fn server_state(
    data: PathBuf,
    credentials_dir: PathBuf,
    policy: PolicyCatalog,
) -> Result<ServerState> {
    Ok(ServerState {
        credentials: Arc::new(CredentialStore::new(credentials_dir)?),
        events: Arc::new(ServerEventStore::open_with_policy(data, policy)?),
    })
}

fn spawn_notification_worker(
    events: Arc<ServerEventStore>,
    dispatcher: Arc<NotificationDispatcher>,
    interval_seconds: u64,
    limit: usize,
) -> Result<()> {
    if interval_seconds == 0 {
        bail!("notify interval must be greater than zero");
    }
    if limit == 0 {
        bail!("notify limit must be greater than zero");
    }
    std::thread::Builder::new()
        .name("beacon-notify".into())
        .spawn(move || loop {
            if let Err(error) = dispatcher.deliver_due(&events, limit) {
                eprintln!("beacon notification worker error: {error:#}");
            }
            std::thread::sleep(std::time::Duration::from_secs(interval_seconds));
        })
        .context("start notification worker")?;
    Ok(())
}

fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/events", post(accept_event))
        .route("/v1/enroll", post(enroll_agent))
        .route("/v1/alerts", get(list_alerts))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state)
}

async fn enroll_agent(
    State(state): State<ServerState>,
    Json(request): Json<EnrollmentRequest>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking({
        let events = state.events.clone();
        let credentials = state.credentials.directory.clone();
        move || events.consume_enrollment(&request.name, &request.code, &credentials)
    })
    .await;
    match result {
        Ok(Ok(token)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "token": token })),
        ),
        Ok(Err(error)) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
    }
}

fn validate_agent_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || name == "." || name == ".." {
        bail!("agent name must be 1-64 characters and cannot be '.' or '..'");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("agent name may contain only ASCII letters, numbers, '.', '_' and '-'");
    }
    Ok(())
}

fn hash_secret(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.trim().as_bytes()))
}

fn write_secret_file(path: &std::path::Path, secret: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("secret file must have a parent directory")?;
    if path.exists() {
        bail!("secret file {} already exists", path.display());
    }
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    use std::io::Write;
    file.write_all(secret.trim().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::hard_link(&temporary, path).with_context(|| {
        format!(
            "install secret file {}; it may already exist",
            path.display()
        )
    })?;
    fs::remove_file(temporary)?;
    Ok(())
}

fn write_credential(path: &std::path::Path, token: &str) -> Result<()> {
    write_secret_file(path, token)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn accept_event(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(event): Json<Event>,
) -> impl IntoResponse {
    if !authorized(&headers, &state.credentials).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        );
    }
    if let Err(error) = event.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        );
    }
    let event_id = event.event_id.clone();
    let result = tokio::task::spawn_blocking({
        let events = state.events.clone();
        move || events.accept(&event)
    })
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
        }
    };
    match result {
        Ok(result) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "status": "accepted",
                "event_id": event_id,
                "duplicate": result.duplicate,
                "alert": result.alert
            })),
        ),
        Err(error) if error.downcast_ref::<EventConflict>().is_some() => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": EventConflict.to_string() })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
    }
}

async fn list_alerts(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<AlertQuery>,
) -> impl IntoResponse {
    if !authorized(&headers, &state.credentials).await {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        );
    }
    let status = match query.status {
        Some(value) => match value.parse::<AlertStatus>() {
            Ok(status) => Some(status),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": error })),
                )
            }
        },
        None => None,
    };
    let result = tokio::task::spawn_blocking({
        let events = state.events.clone();
        move || events.list_alerts(status, query.limit)
    })
    .await;
    match result {
        Ok(Ok(alerts)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "alerts": alerts })),
        ),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        ),
    }
}

async fn authorized(headers: &HeaderMap, credentials: &CredentialStore) -> bool {
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
    else {
        return false;
    };
    let credentials = credentials.clone();
    tokio::task::spawn_blocking(move || credentials.authorized(&token))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Severity;
    use axum::body::Body;
    use axum::http::Request;
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_event(state: EventState, event_id: String) -> Event {
        Event {
            schema_version: 1,
            event_id,
            event_type: "backup.restic.stale".into(),
            source: "test".into(),
            host_id: "backup".into(),
            state,
            severity: Severity::Critical,
            fingerprint: "backup/restic/age".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            facts: BTreeMap::from([("age_hours".into(), serde_json::json!(41))]),
        }
    }

    fn request_for(event: &Event) -> Request<Body> {
        Request::post("/v1/events")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(event).unwrap()))
            .unwrap()
    }

    fn credentials() -> PathBuf {
        let directory = std::env::temp_dir().join(format!("beacon-credentials-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("backup.token"), "test-token\n").unwrap();
        directory
    }

    #[tokio::test]
    async fn health_is_public() {
        let root = std::env::temp_dir().join(format!("beacon-health-{}", Uuid::new_v4()));
        let credentials = credentials();
        let app = router(root.clone(), credentials.clone()).unwrap();
        let response = app
            .clone()
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn firing_is_deduplicated_and_resolved() {
        let root = std::env::temp_dir().join(format!("beacon-lifecycle-{}", Uuid::new_v4()));
        let credentials = credentials();
        let app = router(root.clone(), credentials.clone()).unwrap();
        let fingerprint = "backup/restic/age";
        let firing_one = test_event(EventState::Firing, Uuid::new_v4().to_string());
        let firing_two = test_event(EventState::Firing, Uuid::new_v4().to_string());
        let mut resolved = test_event(EventState::Resolved, Uuid::new_v4().to_string());
        resolved.occurred_at = "2026-01-01T01:00:00Z".into();

        for event in [&firing_one, &firing_two, &resolved] {
            let response = app.clone().oneshot(request_for(event)).await.unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        drop(app);
        let store = ServerEventStore::open(root.clone()).unwrap();
        let alerts = store.list_alerts(Some(AlertStatus::Resolved), 10).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].fingerprint, fingerprint);
        assert_eq!(alerts[0].status, AlertStatus::Resolved);
        assert_eq!(alerts[0].event_count, 3);
        drop(store);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn duplicate_event_is_idempotent_and_conflicting_payload_is_rejected() {
        let root = std::env::temp_dir().join(format!("beacon-idempotency-{}", Uuid::new_v4()));
        let credentials = credentials();
        let app = router(root.clone(), credentials.clone()).unwrap();
        let event_id = Uuid::new_v4().to_string();
        let event = test_event(EventState::Firing, event_id);
        let mut changed = event.clone();
        changed
            .facts
            .insert("age_hours".into(), serde_json::json!(42));

        let first = app.clone().oneshot(request_for(&event)).await.unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let duplicate = app.clone().oneshot(request_for(&event)).await.unwrap();
        assert_eq!(duplicate.status(), StatusCode::ACCEPTED);
        let conflict = app.clone().oneshot(request_for(&changed)).await.unwrap();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);

        drop(app);
        let store = ServerEventStore::open(root.clone()).unwrap();
        let alerts = store.list_alerts(None, 10).unwrap();
        assert_eq!(alerts[0].event_count, 1);
        drop(store);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn alerts_endpoint_requires_auth_and_filters_status() {
        let root = std::env::temp_dir().join(format!("beacon-query-{}", Uuid::new_v4()));
        let credentials = credentials();
        let app = router(root.clone(), credentials.clone()).unwrap();
        let event = test_event(EventState::Firing, Uuid::new_v4().to_string());
        assert_eq!(
            app.clone()
                .oneshot(request_for(&event))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );

        let unauthorized = app
            .clone()
            .oneshot(Request::get("/v1/alerts").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .clone()
            .oneshot(
                Request::get("/v1/alerts?status=firing&limit=10")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drop(response);
        drop(app);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn credentials_can_be_rotated_and_revoked_without_restart() {
        let root = std::env::temp_dir().join(format!("beacon-credentials-{}", Uuid::new_v4()));
        let credentials = credentials();
        let app = router(root.clone(), credentials.clone()).unwrap();
        let event = test_event(EventState::Firing, Uuid::new_v4().to_string());

        let old_token = request_for(&event);
        assert_eq!(
            app.clone().oneshot(old_token).await.unwrap().status(),
            StatusCode::ACCEPTED
        );

        fs::write(credentials.join("backup.token"), "rotated-token\n").unwrap();
        let rejected = app
            .clone()
            .oneshot(
                Request::post("/v1/events")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&event).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let rotated = app
            .clone()
            .oneshot(
                Request::post("/v1/events")
                    .header(header::AUTHORIZATION, "Bearer rotated-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&test_event(
                            EventState::Info,
                            Uuid::new_v4().to_string(),
                        ))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rotated.status(), StatusCode::ACCEPTED);
        drop(app);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn state_survives_store_reopen() {
        let root = std::env::temp_dir().join(format!("beacon-reopen-{}", Uuid::new_v4()));
        let event = test_event(EventState::Firing, Uuid::new_v4().to_string());
        {
            let store = ServerEventStore::open(root.clone()).unwrap();
            store.accept(&event).unwrap();
        }
        {
            let store = ServerEventStore::open(root.clone()).unwrap();
            let alerts = store.list_alerts(None, 10).unwrap();
            assert_eq!(alerts.len(), 1);
            assert_eq!(alerts[0].last_event_id, event.event_id);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enrollment_code_is_single_use_and_creates_credential() {
        let root = std::env::temp_dir().join(format!("beacon-enrollment-{}", Uuid::new_v4()));
        let credentials = credentials();
        let store = ServerEventStore::open(root.clone()).unwrap();
        let code = "one-time-code";
        store.create_enrollment("media", code, 900).unwrap();
        let token = store
            .consume_enrollment("media", code, &credentials)
            .unwrap();
        assert!(!token.is_empty());
        assert!(credentials.join("media.token").exists());
        assert!(store
            .consume_enrollment("media", code, &credentials)
            .is_err());
        drop(store);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enrollment_rejects_expired_code_and_invalid_name() {
        let root =
            std::env::temp_dir().join(format!("beacon-enrollment-expired-{}", Uuid::new_v4()));
        let credentials = credentials();
        let store = ServerEventStore::open(root.clone()).unwrap();
        store
            .create_enrollment("media", "expired-code", 0)
            .unwrap_err();
        store.create_enrollment("media", "valid-code", 1).unwrap();
        assert!(store
            .consume_enrollment("bad/name", "valid-code", &credentials)
            .is_err());
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(store
            .consume_enrollment("media", "valid-code", &credentials)
            .is_err());
        drop(store);
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn enrollment_endpoint_consumes_code_and_returns_token() {
        let root = std::env::temp_dir().join(format!("beacon-enrollment-http-{}", Uuid::new_v4()));
        let credentials = credentials();
        let store = ServerEventStore::open(root.clone()).unwrap();
        store.create_enrollment("media", "http-code", 900).unwrap();
        drop(store);
        let app = router(root.clone(), credentials.clone()).unwrap();
        let response = app
            .oneshot(
                Request::post("/v1/enroll")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "name": "media", "code": "http-code" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(!fs::read_to_string(credentials.join("media.token"))
            .unwrap()
            .trim()
            .is_empty());
        fs::remove_dir_all(credentials).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
