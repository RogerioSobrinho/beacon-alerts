# Beacon Alerts

Beacon is an open-source, distributed event and alert system for infrastructure
and applications.

It is not a check runner, probe system, metrics database, log store, or remote
administration tool. Existing detectors emit normalized events to Beacon. The
server owns alert state, deduplication, policy, templates, and notification
delivery. Telegram is the first planned channel.

```text
local detectors -> beacon agent -> beacon server -> notification channels
                                      |
                                      +-> VictoriaLogs (optional history)
```

## Status

This repository is an early bootstrap. The CLI shape and event contract are
provisional. The current stage implements authenticated HTTP intake, atomic
server-side event persistence, local spooling, and one-shot agent delivery.
Basic alert lifecycle state, policy routing, and a durable notification queue
are now implemented. Telegram delivery is not implemented yet.

## Bootstrap

The project intentionally starts as one binary with subcommands:

```text
beacon server
beacon agent
beacon send
beacon replay
```

The current `send` command renders and durably queues a normalized event in an
atomic local JSON spool. `agent` drains that spool to the authenticated server
and removes an event only after a successful server response.

```text
beacon server --token-file /path/to/server.token --data /var/lib/beacon/events
beacon agent --token-file /path/to/agent.token --spool /var/lib/beacon/spool
beacon server --policy-file /etc/beacon/policy.json
```

The transport bootstrap uses one bearer token per running server. This is not
the final production credential model: per-agent credentials, rotation, TLS,
and authorization scopes are required before deployment outside a controlled
trusted network.

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
- Telegram channel adapter.
- Optional VictoriaLogs event sink.

## Documentation

- [Architecture](docs/architecture.md)
- [Event protocol](docs/protocol.md)
- [Security model](docs/security.md)
- [Operations](docs/operations.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
