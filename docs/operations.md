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
- Validate the policy file before starting the server; keep it free of secrets.
- Validate Telegram config before enabling the in-process notification worker;
  keep the token file separately protected and root-owned.
- Store each agent transport token as a separate root-owned file in the server
  credentials directory with restrictive permissions; do not pass tokens on the
  command line.
- Create agent tokens through the one-time enrollment workflow; do not manually
  copy bearer values between hosts.
- Remove enrollment code files after successful use and verify their short TTL.
- Do not expose the service beyond the trusted management network, even with
  TLS, until the deployment has been validated.
- For non-development operation, configure both server TLS files and the agent
  CA file; never use `--allow-http`.
- Keep the TLS private key readable only by the Beacon server service account.
- Send Beacon stdout/stderr to journald and let `journal-upload` handle
  forwarding to VictoriaLogs; do not add a direct VictoriaLogs client.

## Validation Before Production

- server restart preserves alert state;
- repeated events with the same fingerprint update one alert record;
- a resolved event changes alert state without deleting event history;
- each event/channel pair creates at most one notification job;
- a crashed dispatcher can reclaim stale in-flight jobs;
- agent restart preserves its spool;
- lost acknowledgements do not lose events;
- Telegram failures retry without alert duplication;
- firing and resolved messages are both delivered;
- invalid and sensitive payloads are rejected;
- SQLite backup and restore are tested for the bootstrap deployment;
- Telegram `2xx` responses mark jobs sent; `429`/`5xx` and network errors retry;
- Telegram `4xx` errors are recorded as permanent failures;
- rollback to the previous emitter works for each migrated source.
- two Beacon processes cannot use the same spool or server data directory at
  the same time.
- certificate renewal is tested by replacing the files and restarting the
  service with a rollback copy available;
- SQLite backup is restored to an isolated directory and notification jobs are
  inspected before returning the service to production.
