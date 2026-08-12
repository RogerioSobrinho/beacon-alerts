use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use fs2::FileExt;

use crate::model::Event;

#[derive(Clone)]
pub struct Spool {
    inner: Arc<SpoolInner>,
}

struct SpoolInner {
    pending: PathBuf,
    _lock: File,
}

impl Spool {
    pub fn open(root: PathBuf) -> Result<Self> {
        let pending = root.join("pending");
        fs::create_dir_all(&pending)
            .with_context(|| format!("create spool directory {}", pending.display()))?;
        restrict_directory(&root)?;
        restrict_directory(&pending)?;
        let lock_path = root.join(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("open spool lock {}", lock_path.display()))?;
        lock.try_lock_exclusive().with_context(|| {
            format!(
                "lock spool {}; another Beacon process may be using it",
                root.display()
            )
        })?;
        Ok(Self {
            inner: Arc::new(SpoolInner {
                pending,
                _lock: lock,
            }),
        })
    }

    pub fn enqueue(&self, event: &Event) -> Result<PathBuf> {
        event.validate()?;
        let destination = self.inner.pending.join(format!("{}.json", event.event_id));
        if destination.exists() {
            bail!("event {} is already queued", event.event_id);
        }
        let temporary = self.inner.pending.join(format!(".{}.tmp", event.event_id));
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

    pub fn list(&self) -> Result<Vec<Event>> {
        let mut paths = fs::read_dir(&self.inner.pending)?
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

    pub fn remove(&self, event: &Event) -> Result<()> {
        let path = self.inner.pending.join(format!("{}.json", event.event_id));
        fs::remove_file(&path)
            .with_context(|| format!("remove delivered event {}", event.event_id))?;
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Event, EventState, Severity};
    use std::collections::BTreeMap;
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

    #[test]
    fn enqueue_and_list_round_trip() {
        let root = std::env::temp_dir().join(format!("beacon-spool-{}", Uuid::new_v4()));
        let spool = Spool::open(root.clone()).unwrap();
        let event = test_event();
        let path = spool.enqueue(&event).unwrap();
        assert!(path.exists());
        assert_eq!(spool.list().unwrap(), vec![event.clone()]);
        spool.remove(&event).unwrap();
        assert!(spool.list().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_event_id_is_rejected() {
        let root = std::env::temp_dir().join(format!("beacon-spool-{}", Uuid::new_v4()));
        let spool = Spool::open(root.clone()).unwrap();
        let event = test_event();
        spool.enqueue(&event).unwrap();
        assert!(spool.enqueue(&event).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
