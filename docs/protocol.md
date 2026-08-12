# Event Protocol

The protocol is channel-neutral. Telegram formatting must never be required by
an event producer.

## Event v1

```json
{
  "schema_version": 1,
  "event_id": "uuid",
  "event_type": "backup.restic.stale",
  "source": "restic-age-check",
  "host_id": "backup",
  "state": "firing",
  "severity": "critical",
  "fingerprint": "backup/restic/age",
  "occurred_at": "2026-08-12T12:00:00Z",
  "facts": {
    "age_hours": 41
  }
}
```

Required fields:

- `schema_version`;
- `event_id`;
- `event_type`;
- `source`;
- `host_id`;
- `state`;
- `severity`;
- `fingerprint`;
- `occurred_at`;
- `facts`.

The same fingerprint must be used by a firing event and its resolved event.
Facts are allowlisted by event type. Arbitrary log or secret content must not
be included.

## States

The initial protocol supports `firing`, `resolved`, and `info`. Acknowledgement,
silencing, maintenance, and suppression are server-side lifecycle features and
will be added only after the basic delivery path is reliable.
