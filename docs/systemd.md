# Systemd and TLS

Beacon is not installed by this repository. The following is the intended
shape for a deployment managed by systemd; adapt paths, user names, and hardening
to the target host after reviewing the live system.

## Server

```ini
[Unit]
Description=Beacon alert server
After=network-online.target
Wants=network-online.target

[Service]
User=beacon
Group=beacon
ExecStart=/usr/local/bin/beacon server \
  --bind 0.0.0.0:8787 \
  --data /var/lib/beacon/events \
  --credentials-dir /etc/beacon/agents.d \
  --policy-file /etc/beacon/policy.json \
  --tls-cert /etc/beacon/tls/server.crt \
  --tls-key /etc/beacon/tls/server.key
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/beacon/events
ReadOnlyPaths=/etc/beacon/agents.d /etc/beacon/policy.json /etc/beacon/tls

[Install]
WantedBy=multi-user.target
```

## Notify

Run `beacon notify` as a separate one-shot service or timer. It needs read/write
access to the same SQLite directory and read access to the Telegram config and
token file. Do not put the Telegram token in the unit or command line.

## Certificate Lifecycle

The infrastructure issues and renews certificates. Keep the private key
outside the repository, restrict its permissions, validate the renewed
certificate before restarting Beacon, and retain the previous pair for rollback.

## SQLite Backup

Back up the Beacon data directory using the host's approved SQLite-consistent
backup procedure. Restore into an isolated directory first, start no production
process against it, inspect alert and notification state, and only then perform
a controlled service replacement. Never copy `.lock` files as live authority.
