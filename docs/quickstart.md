# Quickstart

This is the shortest supported path for a Beacon pilot. It assumes:

- the server host already has Docker Compose v2;
- the agent host is Linux x86_64, macOS, or Windows;
- networking/firewall is already approved;
- Telegram credentials are provisioned through the operator's secret procedure.

The server is container-only. The agent is the only native binary published in
GitHub Releases.

## 1. Install And Start The Server

Download `install/beacon-server-install.sh` from the selected release, inspect
it, and run it on the server host with the immutable image reference:

```sh
sudo IMAGE_REF=ghcr.io/rogeriosobrinho/beacon-alerts@sha256:... \
  ./beacon-server-install.sh
```

The installer creates the directories, downloads the pinned Compose file,
writes `.env`, creates an empty policy only if absent, and validates Compose.
Place `policy.json`, `telegram.json`, and `telegram.token` under `/etc/beacon`
through the approved secret procedure. Do not put secret values in `.env` or Git.

Then run:

```sh
sudo docker compose --env-file /opt/beacon/.env -f /opt/beacon/compose.yaml config --quiet
sudo docker compose --env-file /opt/beacon/.env -f /opt/beacon/compose.yaml pull
sudo docker compose --env-file /opt/beacon/.env -f /opt/beacon/compose.yaml up -d
```

The server exposes `8787` only to approved agent hosts. Plain HTTP requires
`--allow-http` and is for a trusted internal pilot only.

## 2. Install And Enroll One Agent

Download the matching `beacon-agent` release asset, extract it, and run the
installer on the agent host. Use the existing least-privilege service account:

```sh
sudo SERVICE_USER=media SERVER_URL=http://SERVER:8787 ALLOW_HTTP=1 \
  ./beacon-agent-install.sh
```

The installer creates the binary, spool, hardened service, and periodic timer.
It does not create users, read secrets, change firewall rules, or enroll the
host.

Create a short-lived code on the server:

```sh
sudo docker compose --env-file /opt/beacon/.env -f /opt/beacon/compose.yaml \
  run --rm beacon agent create --name media \
  --code-file /etc/beacon/enrollment/media.code --ttl-seconds 900
```

Transfer the code file through the approved operator path. The code is a
bootstrap secret; do not print it, commit it, or send it through an unapproved
channel. On the agent host:

```sh
beacon-agent enroll \
  --server http://SERVER:8787 --allow-http \
  --name media --code-file /path/to/media.code \
  --token-file /etc/beacon/agent.token
```

The agent creates its token with mode `0600`. Delete the code file immediately
after success. For TLS, use `https://`, omit `--allow-http`, and add `--ca-file`.

## 3. Enable Periodic Delivery

`beacon-agent run` is a one-shot drain. On systemd hosts, use a timer:

```ini
[Unit]
Description=Run Beacon event agent periodically

[Timer]
OnBootSec=30s
OnUnitActiveSec=60s
Persistent=true
Unit=beacon-agent.service

[Install]
WantedBy=timers.target
```

The service must run as the existing least-privilege application user, not as
root. Enable the timer after installing the service unit:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now beacon-agent.timer
```

## Send A Test

```sh
beacon-agent send \
  --event-type beacon.test --source manual --host media \
  --state firing --severity warning \
  --fingerprint beacon/test/media \
  --facts '{"message":"pilot firing"}' \
  --spool /var/lib/beacon/spool
```

Validate the real workflow: the spool file disappears after acknowledgement,
the alert appears in `GET /v1/alerts`, and the configured notification arrives.
Send a matching `resolved` event to validate recovery.

## Why There Is No Portal Yet

Beacon does not currently ship a setup portal. A portal exposed on the same
HTTP listener would make enrollment and Telegram configuration high-value
unauthenticated targets, especially in the homelab pilot. The safer next step is
a local-only administrator workflow, such as a CLI or a portal bound to
localhost and accessed through an authenticated SSH tunnel, with explicit
separation from the event API.

Until that exists, this quickstart is the supported path. The detailed
configuration, security, and rollback procedures remain in
`docs/configuration.md`, `docs/security.md`, and `docs/homelab-pilot.md`.
