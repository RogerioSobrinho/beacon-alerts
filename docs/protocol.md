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

The server reads one bearer token per agent from the configured credentials
directory. Replacing or removing a token file takes effect on the next request,
enabling rotation and revocation without restart. TLS and authorization scopes
are required before production use.

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

## Policy Catalog

The server optionally reads a JSON policy catalog with `--policy-file`. Each
rule may match `event_type`, `source`, `host_id`, `state`, and `severity`, and
must name one or more channels. Matching rules are combined and duplicate
channel names are removed. Disabled rules are ignored. See
`docs/policy.example.json`.

For each accepted event and matching channel, the server creates one durable
notification job in the same SQLite transaction as the event and alert. Jobs
use the unique key `(event_id, channel)`, so event retries cannot enqueue a
duplicate. Delivery states are `pending`, `in_flight`, `sent`, and `failed`.
Failed jobs receive exponential retry timestamps and stale `in_flight` jobs
can be reclaimed after a process crash.

## Telegram Delivery

The `beacon notify` command is a one-shot worker. It reads Telegram settings
from JSON and the bot token from the configured token file, then drains due
jobs. It uses HTTPS and a bounded request timeout. Tokens are never accepted
as command-line values or included in job payloads.

## Operational Logs

Beacon writes operational logs to stdout/stderr for systemd-journald. The
`journal-upload` workflow may forward those logs to VictoriaLogs. Beacon does
not send alert records directly to VictoriaLogs. A separate detector may query
VictoriaLogs and submit a normalized event to Beacon.

## TLS Transport

The server accepts `--tls-cert` and `--tls-key` PEM files as a pair. Agents use
`--ca-file` to validate the server certificate and must use an `https://` URL.
Hostname verification remains enabled by the HTTP client. `--allow-http` is
available only as an explicit local-development override.
