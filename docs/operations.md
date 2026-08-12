# Operations

Beacon is not deployed to the homelab yet. This document defines the intended
operational constraints.

## Deployment

- Pin every release and container image by version or digest.
- Validate configuration before starting the server.
- Run the server as an unprivileged user.
- Do not grant Docker socket access.
- Keep the server reachable only from approved agent hosts.
- Store PostgreSQL and channel secrets outside the repository.

## Validation Before Production

- server restart preserves alert state;
- agent restart preserves its spool;
- lost acknowledgements do not lose events;
- Telegram failures retry without alert duplication;
- firing and resolved messages are both delivered;
- invalid and sensitive payloads are rejected;
- PostgreSQL backup and restore are tested;
- rollback to the previous emitter works for each migrated source.
