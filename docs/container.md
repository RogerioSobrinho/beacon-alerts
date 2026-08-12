# Container Deployment

The Beacon server is packaged as a non-root container. Agents remain native
systemd services on producer VMs so they can observe local timers and services
without Docker socket access.

## Build And Publish

The `image.yml` workflow publishes only versioned tags such as `v0.1.0-rc2` to
GHCR. It does not publish `latest`. The Compose file consumes the full image
reference through `BEACON_IMAGE_REF`; production should set this to the
resulting image digest, not a moving tag.

The complete user configuration procedure is in `docs/configuration.md`.

## Ops Compose

`compose.yaml` is intended for the Ops VM after its paths, firewall, and service
user have been validated live. It uses HTTP with `--allow-http` because this
homelab has no TLS infrastructure. The listener is bound to the Ops address and
the firewall must restrict port `8787` to approved agent hosts.

The server and notification worker run in the same container and share one
SQLite lock. Do not run a second server container against the same data
directory. The worker is enabled by `--telegram-config` and reads the token from
the path inside that configuration file.

## Secrets And Volumes

- `/var/lib/beacon/events`: writable SQLite data directory;
- `/etc/beacon/agents.d`: one individual agent token file per agent;
- `/etc/beacon/policy.json`: routing policy without secrets;
- `/etc/beacon/telegram.json`: Telegram configuration without the bot token;
- `/etc/beacon/telegram.token`: Telegram bot token, read-only in the container.

No credentials are committed to the repository. The container has a read-only
root filesystem, drops all Linux capabilities, and does not mount the Docker
socket.

## Validation

Before starting on a host:

```sh
docker compose --env-file .env config --quiet
docker compose --env-file .env pull
docker compose --env-file .env up -d
docker compose --env-file .env ps
```

Use the exact image digest selected for the pilot, not an unpinned moving tag.
