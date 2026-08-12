#![allow(clippy::unused_async)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use fs2::FileExt;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::net::TcpListener;

use crate::model::Event;

#[derive(Debug)]
pub struct ServerConfig {
    pub bind: String,
    pub data: PathBuf,
    pub token: String,
}

#[derive(Clone)]
pub struct ServerState {
    token: Arc<String>,
    events: Arc<ServerEventStore>,
}

struct ServerEventStore {
    accepted: PathBuf,
    commit_lock: Mutex<()>,
    _lock: std::fs::File,
}

#[derive(Debug, Error)]
#[error("event_id already exists with a different payload")]
struct EventConflict;

enum AcceptResult {
    New,
    Duplicate,
}

impl ServerEventStore {
    fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("create server data directory {}", root.display()))?;
        let accepted = root.join("accepted");
        fs::create_dir_all(&accepted)
            .with_context(|| format!("create accepted event directory {}", accepted.display()))?;
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
        Ok(Self {
            accepted,
            commit_lock: Mutex::new(()),
            _lock: lock,
        })
    }

    fn accept(&self, event: &Event) -> Result<AcceptResult> {
        event.validate()?;
        let _lock = self
            .commit_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        let destination = self.accepted.join(format!("{}.json", event.event_id));
        if destination.exists() {
            let existing: Event = serde_json::from_reader(
                fs::File::open(&destination)
                    .with_context(|| format!("open existing event {}", event.event_id))?,
            )
            .with_context(|| format!("decode existing event {}", event.event_id))?;
            if existing != *event {
                return Err(EventConflict.into());
            }
            return Ok(AcceptResult::Duplicate);
        }
        let temporary = self.accepted.join(format!(".{}.tmp", event.event_id));
        let encoded = serde_json::to_vec_pretty(event)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .with_context(|| format!("create temporary accepted event {}", event.event_id))?;
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &destination)
            .with_context(|| format!("commit accepted event {}", event.event_id))?;
        Ok(AcceptResult::New)
    }
}

pub async fn serve(args: ServerConfig) -> Result<()> {
    if args.token.trim().is_empty() {
        bail!("server token cannot be empty");
    }
    let bind: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("invalid server bind address {}", args.bind))?;
    let router = router(args.data, args.token)?;
    let listener = TcpListener::bind(bind).await?;
    println!("Beacon server listening on {}", listener.local_addr()?);
    serve_listener(listener, router).await?;
    Ok(())
}

pub fn router(data: PathBuf, token: String) -> Result<Router> {
    if token.trim().is_empty() {
        bail!("server token cannot be empty");
    }
    Ok(build_router(ServerState {
        token: Arc::new(token),
        events: Arc::new(ServerEventStore::open(data)?),
    }))
}

pub async fn serve_listener(listener: TcpListener, router: Router) -> Result<()> {
    axum::serve(listener, router).await?;
    Ok(())
}

fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/events", post(accept_event))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn accept_event(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(event): Json<Event>,
) -> impl IntoResponse {
    if !authorized(&headers, state.token.as_str()) {
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
        Ok(AcceptResult::New | AcceptResult::Duplicate) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "status": "accepted",
                "event_id": event_id
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

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token.as_bytes().ct_eq(expected.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_event() -> Event {
        Event {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            event_type: "backup.restic.stale".into(),
            source: "test".into(),
            host_id: "backup".into(),
            state: crate::model::EventState::Firing,
            severity: crate::model::Severity::Critical,
            fingerprint: "backup/restic/age".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            facts: BTreeMap::from([("age_hours".into(), serde_json::json!(41))]),
        }
    }

    #[tokio::test]
    async fn health_is_public() {
        let root = std::env::temp_dir().join(format!("beacon-health-{}", Uuid::new_v4()));
        let app = router(root.clone(), "test-token".into()).unwrap();
        let response = app
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn event_requires_auth_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("beacon-server-{}", Uuid::new_v4()));
        let app = router(root.clone(), "test-token".into()).unwrap();
        let event = test_event();
        let body = serde_json::to_vec(&event).unwrap();

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/v1/events")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/v1/events")
                        .header(header::AUTHORIZATION, "Bearer test-token")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        assert!(root
            .join("accepted")
            .join(format!("{}.json", event.event_id))
            .exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn reused_event_id_with_changed_payload_is_conflict() {
        let root = std::env::temp_dir().join(format!("beacon-conflict-{}", Uuid::new_v4()));
        let app = router(root.clone(), "test-token".into()).unwrap();
        let event = test_event();
        let mut changed = event.clone();
        changed
            .facts
            .insert("age_hours".into(), serde_json::json!(42));

        for payload in [event, changed] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/v1/events")
                        .header(header::AUTHORIZATION, "Bearer test-token")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            if response.status() == StatusCode::ACCEPTED {
                continue;
            }
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
