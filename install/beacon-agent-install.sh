#!/usr/bin/env bash
set -euo pipefail

PREFIX=${PREFIX:-/usr/local/bin}
CONFIG_DIR=${CONFIG_DIR:-/etc/beacon}
SPOOL_DIR=${SPOOL_DIR:-/var/lib/beacon/spool}
SERVICE_USER=${SERVICE_USER:-beacon}
SERVER_URL=${SERVER_URL:-https://127.0.0.1:8787}
ALLOW_HTTP=${ALLOW_HTTP:-0}

if [[ ${EUID} -ne 0 ]]; then
  echo "run as root (sudo $0)" >&2
  exit 1
fi

if ! id "${SERVICE_USER}" >/dev/null 2>&1; then
  echo "service user ${SERVICE_USER} does not exist; set SERVICE_USER to an existing least-privilege account" >&2
  exit 1
fi

install -d -m 0755 "${PREFIX}"
install -d -o "${SERVICE_USER}" -g "${SERVICE_USER}" -m 0750 "${SPOOL_DIR}"
install -d -o root -g "${SERVICE_USER}" -m 0770 "${CONFIG_DIR}"
install -m 0755 beacon-agent "${PREFIX}/beacon-agent"

agent_args=(run --server "${SERVER_URL}" --spool "${SPOOL_DIR}" --token-file "${CONFIG_DIR}/agent.token")
if [[ ${ALLOW_HTTP} == 1 ]]; then
  agent_args+=(--allow-http)
fi

cat > /etc/systemd/system/beacon-agent.service <<EOF
[Unit]
Description=Beacon event agent
After=network-online.target
Wants=network-online.target

[Service]
User=${SERVICE_USER}
Group=${SERVICE_USER}
ExecStart=${PREFIX}/beacon-agent ${agent_args[*]}
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${SPOOL_DIR}
ReadOnlyPaths=${CONFIG_DIR}/agent.token

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/beacon-agent.timer <<'EOF'
[Unit]
Description=Run Beacon event agent periodically

[Timer]
OnBootSec=30s
OnUnitActiveSec=60s
Persistent=true
Unit=beacon-agent.service

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl disable beacon-agent.service >/dev/null 2>&1 || true
systemctl enable --now beacon-agent.timer
echo "installed ${PREFIX}/beacon-agent; timer enabled"
