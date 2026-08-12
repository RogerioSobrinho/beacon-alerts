# Systemd and TLS

Beacon agents are intended to run as native systemd services on event-producing
VMs. The central server is packaged as a container; see
`docs/container.md`. Adapt paths, user names, and hardening to the target host
after reviewing the live system.

## Agent

```ini
[Unit]
Description=Beacon event agent
After=network-online.target
Wants=network-online.target

[Service]
User=beacon
Group=beacon
ExecStart=/usr/local/bin/beacon agent \
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

## Notification Worker

For the container deployment, the notification worker runs inside the server
process when `--telegram-config` is supplied. Do not run a second `beacon notify`
process against the same SQLite directory. The standalone `notify` command is
available for controlled one-shot operation when the server is stopped.

## Certificate Lifecycle

The infrastructure issues and renews certificates. Keep the private key
outside the repository, restrict its permissions, validate the renewed
certificate before restarting Beacon, and retain the previous pair for rollback.

## SQLite Backup

Back up the Beacon data directory using the host's approved SQLite-consistent
backup procedure. Restore into an isolated directory first, start no production
process against it, inspect alert and notification state, and only then perform
a controlled service replacement. Never copy `.lock` files as live authority.
