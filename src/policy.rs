use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Event, EventState, Severity};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PolicyCatalog {
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PolicyRule {
    pub name: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub event_type: Option<String>,
    pub source: Option<String>,
    pub host_id: Option<String>,
    pub state: Option<EventState>,
    pub severity: Option<Severity>,
    pub channels: Vec<String>,
}

fn enabled_by_default() -> bool {
    true
}

impl PolicyCatalog {
    pub fn validate(&self) -> Result<()> {
        if self.rules.len() > 256 {
            bail!("policy catalog cannot contain more than 256 rules");
        }
        for rule in &self.rules {
            if rule.name.trim().is_empty() || rule.name.len() > 128 {
                bail!("policy name must be between 1 and 128 bytes");
            }
            if rule.channels.is_empty() || rule.channels.len() > 16 {
                bail!(
                    "policy '{}' must contain between 1 and 16 channels",
                    rule.name
                );
            }
            for channel in &rule.channels {
                if channel.trim().is_empty() || channel.len() > 64 {
                    bail!("policy channel must be between 1 and 64 bytes");
                }
            }
        }
        Ok(())
    }

    pub fn channels_for(&self, event: &Event) -> Vec<String> {
        let mut channels = Vec::new();
        for rule in &self.rules {
            if rule.enabled && rule.matches(event) {
                for channel in &rule.channels {
                    if !channels.iter().any(|known| known == channel) {
                        channels.push(channel.clone());
                    }
                }
            }
        }
        channels
    }
}

impl PolicyRule {
    fn matches(&self, event: &Event) -> bool {
        self.event_type
            .as_deref()
            .is_none_or(|value| value == event.event_type)
            && self
                .source
                .as_deref()
                .is_none_or(|value| value == event.source)
            && self
                .host_id
                .as_deref()
                .is_none_or(|value| value == event.host_id)
            && self
                .state
                .as_ref()
                .is_none_or(|value| value == &event.state)
            && self
                .severity
                .as_ref()
                .is_none_or(|value| value == &event.severity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn event() -> Event {
        Event {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            event_type: "backup.restic.stale".into(),
            source: "restic-age-check".into(),
            host_id: "backup".into(),
            state: EventState::Firing,
            severity: Severity::Critical,
            fingerprint: "backup/restic/age".into(),
            occurred_at: "2026-01-01T00:00:00Z".into(),
            facts: BTreeMap::new(),
        }
    }

    #[test]
    fn matching_rules_are_combined_without_duplicate_channels() {
        let catalog = PolicyCatalog {
            rules: vec![
                PolicyRule {
                    name: "critical-backup".into(),
                    enabled: true,
                    event_type: Some("backup.restic.stale".into()),
                    source: None,
                    host_id: Some("backup".into()),
                    state: Some(EventState::Firing),
                    severity: Some(Severity::Critical),
                    channels: vec!["telegram".into(), "audit".into()],
                },
                PolicyRule {
                    name: "all-backup".into(),
                    enabled: true,
                    event_type: None,
                    source: None,
                    host_id: Some("backup".into()),
                    state: None,
                    severity: None,
                    channels: vec!["telegram".into()],
                },
            ],
        };
        assert_eq!(catalog.channels_for(&event()), ["telegram", "audit"]);
    }
}
