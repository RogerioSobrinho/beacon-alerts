# Beacon Alerts

Beacon is an open-source, distributed event and alert system for infrastructure
and applications.

It is not a check runner, probe system, metrics database, log store, or remote
administration tool. Existing detectors emit normalized events to Beacon. The
server owns alert state, deduplication, policy, templates, and notification
delivery. Telegram is the first planned channel.

```text
detectors / automations -> beacon agent -> beacon server -> notification channels
       |                         |                 |
       |                         |                 +-> journal -> journal-upload -> VictoriaLogs
       +-> query VictoriaLogs (optional detector input)
```

## Status

This repository is an early bootstrap. The CLI shape and event contract are
provisional. The current stage implements authenticated HTTP intake, atomic
server-side event persistence, local spooling, and one-shot agent delivery.
Basic alert lifecycle state, policy routing, and a durable notification queue
are now implemented, including a one-shot Telegram delivery worker. It does
not run automatically.

## Bootstrap

The project intentionally starts as one binary with subcommands:

```text
beacon server
beacon agent
beacon send
beacon replay
beacon notify
```

The current `send` command renders and durably queues a normalized event in an
atomic local JSON spool. `agent` drains that spool to the authenticated server
and removes an event only after a successful server response.

```text
beacon server --credentials-dir /etc/beacon/agents.d --data /var/lib/beacon/events
beacon agent --token-file /path/to/agent.token --spool /var/lib/beacon/spool
beacon server --policy-file /etc/beacon/policy.json
beacon notify --data /var/lib/beacon/events --telegram-config /etc/beacon/telegram.json
```

The server accepts one bearer token file per agent in `--credentials-dir`.
Adding, replacing, or removing a file rotates or revokes that agent without a
server restart. TLS and authorization scopes remain required before deployment
outside a controlled trusted network.

## Design Principles

- Keep detection close to the system that owns the relevant state.
- Centralize alert lifecycle and notification presentation.
- Never put Telegram credentials on agents.
- Keep Telegram messages short, actionable, and safe for semi-public delivery.
- Persist events before acknowledging them.
- Do not execute commands on clients from the server.
- Make the protocol versioned and usable by non-Rust clients.

## Planned Components

- `beacon agent`: local client with durable spool and retry.
- `beacon server`: authenticated event intake and alert lifecycle.
- `beacon send`: local CLI for scripts and operators.
- `beacon replay`: local spool inspection and controlled replay.
- `GET /v1/alerts`: authenticated inspection of persisted alert state.
- `docs/policy.example.json`: example event-to-channel routing catalog.
- `beacon notify`: one-shot notification delivery worker.
- `docs/telegram.example.json`: secret-free Telegram configuration example.
- VictoriaLogs remains a log store and optional detector input, not a Beacon
  alert sink. Beacon writes operational logs to the journal; `journal-upload`
  forwards them to VictoriaLogs.

## Documentation

- [Architecture](docs/architecture.md)
- [Event protocol](docs/protocol.md)
- [Security model](docs/security.md)
- [Operations](docs/operations.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
