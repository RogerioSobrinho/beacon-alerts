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

## Transport v1

The bootstrap server exposes:

- `GET /healthz`: unauthenticated liveness response;
- `POST /v1/events`: authenticated JSON event intake.
- `GET /v1/alerts`: authenticated alert-state inspection.

Clients send `Authorization: Bearer <agent-token>` and an `Event v1` JSON body.
The server validates and atomically stores the event and its alert-state
transition before returning `202 Accepted`. Repeating the same `event_id` with
the same payload is idempotent and returns `202` without incrementing alert
history. Reusing an `event_id` with a different payload returns `409 Conflict`.

The current bootstrap reads the bearer token from a local file and does not
accept it as a command-line value. It uses one configured bearer token for the
server. Per-agent credentials, rotation, TLS configuration, and authorization
scopes are required before production use.

## Alert Lifecycle

The protocol supports `firing`, `resolved`, and `info`. The server maintains one
alert record per `fingerprint`:

- `firing` opens an alert or updates an existing firing alert;
- `resolved` marks the matching alert resolved;
- `info` records an informational state unless the fingerprint is already
  firing.

Different events with the same fingerprint update one record and increment its
event count. The matching resolved event must use the same fingerprint as the
firing event. `GET /v1/alerts` accepts `status=firing`, `status=resolved`, or
`status=info`, plus a bounded `limit` query parameter. Acknowledgement,
silencing, maintenance, and suppression are not implemented yet.
