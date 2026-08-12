# Beacon Alerts

Beacon is an open-source, distributed event and alert system for infrastructure
and applications.

It is not a check runner, probe system, metrics database, log store, or remote
administration tool. Existing detectors emit normalized events to Beacon. The
server owns alert state, deduplication, policy, templates, and notification
delivery. Telegram is the first planned channel.

```text
detectors / automations -> beacon-agent -> beacon-server -> notification channels
       |                         |                 |
       |                         |                 +-> journal -> journal-upload -> VictoriaLogs
       +-> query VictoriaLogs (optional detector input)
```

## Status

This repository contains an MVP release candidate. It implements authenticated
HTTP intake, atomic server-side event persistence, local spooling, alert
lifecycle state, policy routing, a durable notification queue, and a Telegram
worker. The server and native agent still require environment-specific
configuration before deployment.

## Bootstrap

The project provides separate server and agent binaries:

```text
beacon-server run
beacon-server agent create
beacon-agent enroll
beacon-agent run
beacon-agent send
beacon-agent replay
```

The `beacon-agent send` command renders and durably queues a normalized event in
an atomic local JSON spool. `beacon-agent run` drains that spool to the
authenticated server and removes an event only after a successful response.

For the shortest supported setup, see the [Quickstart](docs/quickstart.md).
The [Configuration Guide](docs/configuration.md) contains the complete
directory, permission, Telegram, rollback, and security reference.

### Quick Start

The server container and each native agent need separate configuration. The
server stores its SQLite state in `/var/lib/beacon/events`; agent credentials
are files under `/etc/beacon/agents.d`; policy and Telegram configuration live
under `/etc/beacon`. Never commit these paths' secrets.

GitHub Releases publish `beacon-agent` assets for Linux x86_64, macOS x86_64,
macOS arm64, and Windows x86_64. The `beacon-server` is published only as the
versioned GHCR container image.

The reviewed installers in [`install/`](install/) automate the repetitive
directory, permission, Compose, binary, and systemd setup. They intentionally
leave secrets, firewall changes, enrollment, and service start as explicit
operator actions.

```sh
cp .env.example /opt/beacon/.env
# Edit BEACON_IMAGE_REF to the reviewed image digest.
docker compose --env-file /opt/beacon/.env config --quiet
docker compose --env-file /opt/beacon/.env pull
docker compose --env-file /opt/beacon/.env up -d
```

On a producer host, run the enrollment workflow from the configuration guide,
then configure the native agent with the generated token file:

```sh
beacon-agent run --server http://server.example:8787 --allow-http \
  --spool /var/lib/beacon/spool --token-file /etc/beacon/agent.token
```

For the complete directory layout, permissions, policy, Telegram files,
systemd unit, validation and rollback procedure, read
`docs/configuration.md` before starting a deployment.

```text
beacon-server run --credentials-dir /etc/beacon/agents.d --data /var/lib/beacon/events \
  --tls-cert /etc/beacon/tls/server.crt --tls-key /etc/beacon/tls/server.key
beacon-agent run --server https://beacon.example.internal:8787 \
  --ca-file /etc/beacon/tls/ca.crt --token-file /path/to/agent.token \
  --spool /var/lib/beacon/spool
beacon-server run --policy-file /etc/beacon/policy.json
```

The server accepts one bearer token file per agent in `--credentials-dir`.
Create those credentials with a one-time code using `beacon-server agent create`
and `beacon-agent enroll`. Adding, replacing, or removing a file rotates or
revokes that agent without a server restart. TLS and authorization scopes remain
required before deployment outside a controlled trusted network.

TLS certificate issuance and renewal belong to the deployment infrastructure.
Beacon reads the certificate, private key, and CA files but does not create or
renew them. Plain HTTP requires `--allow-http` on both server and agent and is
intended only for local development.

## Design Principles

- Keep detection close to the system that owns the relevant state.
- Centralize alert lifecycle and notification presentation.
- Never put Telegram credentials on agents.
- Keep Telegram messages short, actionable, and safe for semi-public delivery.
- Persist events before acknowledging them.
- Do not execute commands on clients from the server.
- Make the protocol versioned and usable by non-Rust clients.

## Planned Components

- `beacon-agent`: local client with enrollment, durable spool, and retry.
- `beacon-server`: authenticated event intake and alert lifecycle.
- `beacon-agent send`: local CLI for scripts and operators.
- `beacon-agent replay`: local spool inspection and controlled replay.
- `GET /v1/alerts`: authenticated inspection of persisted alert state.
- `docs/policy.example.json`: example event-to-channel routing catalog.
- `beacon-server run`: includes the notification worker when configured.
- `docs/telegram.example.json`: secret-free Telegram configuration example.
- VictoriaLogs remains a log store and optional detector input, not a Beacon
  alert sink. Beacon writes operational logs to the journal; `journal-upload`
  forwards them to VictoriaLogs.

## Documentation

- [Architecture](docs/architecture.md)
- [Quickstart](docs/quickstart.md)
- [Event protocol](docs/protocol.md)
- [Security model](docs/security.md)
- [Operations](docs/operations.md)
- [Systemd and TLS](docs/systemd.md)
- [Container deployment](docs/container.md)
- [Configuration guide](docs/configuration.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
