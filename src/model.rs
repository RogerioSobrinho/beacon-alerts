use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventState {
    Firing,
    Resolved,
    Info,
}

impl FromStr for EventState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "firing" => Ok(Self::Firing),
            "resolved" => Ok(Self::Resolved),
            "info" => Ok(Self::Info),
            _ => Err(format!("invalid event state: {value}")),
        }
    }
}

impl fmt::Display for EventState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl EventState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
            Self::Info => "info",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "critical" => Ok(Self::Critical),
            "warning" => Ok(Self::Warning),
            "info" => Ok(Self::Info),
            _ => Err(format!("invalid severity: {value}")),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Event {
    pub schema_version: u16,
    pub event_id: String,
    pub event_type: String,
    pub source: String,
    pub host_id: String,
    pub state: EventState,
    pub severity: Severity,
    pub fingerprint: String,
    pub occurred_at: String,
    pub facts: BTreeMap<String, serde_json::Value>,
}

impl Event {
    pub fn validate(&self) -> Result<()> {
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
        assert_eq!(decoded, event);
        decoded.validate().unwrap();
    }

    #[test]
    fn invalid_schema_identity_and_timestamp_are_rejected() {
        let mut event = test_event();
        event.schema_version = 2;
        assert!(event.validate().is_err());

        let mut event = test_event();
        event.event_id = "../../outside-spool".into();
        assert!(event.validate().is_err());

        let mut event = test_event();
        event.occurred_at = "not-a-timestamp".into();
        assert!(event.validate().is_err());
    }
}
