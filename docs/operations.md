# Operations

Beacon is not deployed to the homelab yet. This document defines the intended
operational constraints.

## Deployment

- Pin every release and container image by version or digest.
- Validate configuration before starting the server.
- Run the server as an unprivileged user.
- Do not grant Docker socket access.
- Keep the server reachable only from approved agent hosts.
- Store the SQLite database and channel secrets outside the repository.
- Store server and agent transport tokens in root-owned files with restrictive
  permissions; do not pass them on the command line.
- Do not expose the bootstrap HTTP endpoint beyond the trusted management
  network. It has no TLS or per-agent authorization yet.

## Validation Before Production

- server restart preserves alert state;
- repeated events with the same fingerprint update one alert record;
- a resolved event changes alert state without deleting event history;
- agent restart preserves its spool;
- lost acknowledgements do not lose events;
- Telegram failures retry without alert duplication;
- firing and resolved messages are both delivered;
- invalid and sensitive payloads are rejected;
- PostgreSQL backup and restore are tested;
- rollback to the previous emitter works for each migrated source.
- two Beacon processes cannot use the same spool or server data directory at
  the same time.
