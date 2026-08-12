# Installers

The installers remove repetitive directory, permission, binary, and systemd
setup. They are idempotent for the files they own.

## Agent

Download the matching `beacon-agent` asset, extract it, then run as root:

```sh
sudo SERVICE_USER=media SERVER_URL=http://192.168.68.62:8787 ALLOW_HTTP=1 \
  ./beacon-agent-install.sh
```

The installer does not create a service account, read or create a token, enroll
the host, change firewall rules, or start a one-shot service. It enables the
periodic timer. Enrollment remains an explicit step because its code is a
bootstrap secret.

## Server

Run on the server host with an immutable image digest:

```sh
sudo IMAGE_REF=ghcr.io/rogeriosobrinho/beacon-alerts@sha256:... \
  ./beacon-server-install.sh
```

The server installer creates directories, downloads the pinned Compose file,
writes `.env`, creates an empty policy only if absent, and validates Compose. It
does not create Telegram secrets, change firewall rules, or start the stack.

Starting the stack remains an explicit operator action:

```sh
sudo docker compose --env-file /opt/beacon/.env \
  -f /opt/beacon/compose.yaml up -d
```

Never pipe an installer from an unreviewed URL to a shell. Download the release,
inspect the script, verify its checksum or commit, then execute it.
