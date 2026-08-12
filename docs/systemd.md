# Systemd and TLS

Beacon agents are intended to run as a native one-shot systemd service invoked
by a timer on event-producing VMs. The central server is packaged as a
container; see
`docs/container.md`. Adapt paths and hardening to the target host after
reviewing the live system. For the homelab Media VM, the existing service
account is `media`; do not create a new privileged account just for Beacon.

## Agent

```ini
[Unit]
Description=Beacon event agent
After=network-online.target
Wants=network-online.target

[Service]
User=media
Group=media
ExecStart=/usr/local/bin/beacon-agent run \
  --server http://192.168.68.62:8787 \
  --allow-http \
  --spool /var/lib/beacon/spool \
  --token-file /etc/beacon/agent.token
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/beacon/spool
ReadOnlyPaths=/etc/beacon/agent.token

[Install]
WantedBy=multi-user.target
```

The service exits successfully after draining the current spool. Enable
`beacon-agent.timer`, not the service, for periodic delivery:

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

## Notification Worker

For the container deployment, the notification worker runs inside the
`beacon-server` process when `--telegram-config` is supplied. There is no
standalone notification binary in the split client/server release.

## Certificate Lifecycle

The infrastructure issues and renews certificates. Keep the private key
outside the repository, restrict its permissions, validate the renewed
certificate before restarting Beacon, and retain the previous pair for rollback.

## SQLite Backup

Back up the Beacon data directory using the host's approved SQLite-consistent
backup procedure. Restore into an isolated directory first, start no production
process against it, inspect alert and notification state, and only then perform
a controlled service replacement. Never copy `.lock` files as live authority.
