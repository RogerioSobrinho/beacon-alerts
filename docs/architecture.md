# Architecture

Beacon has two primary runtime roles: an agent on the event-producing host and
a server on the central operations host.

```text
scripts, systemd, applications
              |
              v
       beacon agent
       local durable spool
              |
              v
       beacon server
       event + alert state
       policy + templates
              |
              v
       Telegram adapter
```

## Out of Scope

Beacon does not run checks or probes. Kuma remains responsible for availability
checks. VictoriaLogs remains responsible for log storage and investigation.
Beacon does not collect metrics or execute remote commands.

## Delivery Model

1. A detector creates an event through the local CLI or client API.
2. The agent validates and persists the event in its local spool.
3. The agent sends the event to the server over authenticated transport.
4. The server validates the schema and commits the event before acknowledging it.
5. The server derives or updates alert state using the event fingerprint.
6. A notification worker renders and delivers channel messages.
7. Delivery results remain durable and are retried without creating a new alert.

The current implementation covers steps 1 through 5 with a local JSON spool,
authenticated HTTP intake, SQLite event persistence, and alert state derived by
fingerprint. Notification workers and channel delivery are subsequent stages.

## Initial Persistence

- Server bootstrap: bundled SQLite for event and alert state in one transaction.
- Future production scale: PostgreSQL remains an option for event, alert,
  notification, client, and policy state if the deployment needs it.
- Agent bootstrap: atomic JSON files in a local spool, independent from the
  server database. SQLite remains a possible implementation if queue metadata
  requires it.
- Catalog: versioned configuration reviewed with the source code.
