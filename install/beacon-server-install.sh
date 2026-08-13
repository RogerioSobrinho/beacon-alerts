#!/usr/bin/env bash
set -euo pipefail

ROOT=${ROOT:-/opt/beacon}
DATA_DIR=${DATA_DIR:-/var/lib/beacon/events}
CONFIG_DIR=${CONFIG_DIR:-/etc/beacon}
IMAGE_REF=${IMAGE_REF:?set IMAGE_REF to an immutable Beacon image digest}
RELEASE_TAG=${RELEASE_TAG:-v0.1.0-rc6}
COMPOSE_URL=${COMPOSE_URL:-https://raw.githubusercontent.com/RogerioSobrinho/beacon-alerts/${RELEASE_TAG}/compose.yaml}

if [[ ${EUID} -ne 0 ]]; then
  echo "run as root (sudo $0)" >&2
  exit 1
fi

install -d -o 10001 -g 10001 -m 0750 "${DATA_DIR}"
install -d -o root -g 10001 -m 0770 "${CONFIG_DIR}/agents.d"
install -d -o root -g 10001 -m 0770 "${CONFIG_DIR}/enrollment"
install -d -o root -g root -m 0750 "${ROOT}"

if [[ ! -e ${ROOT}/compose.yaml ]]; then
  curl --fail --silent --show-error --location "${COMPOSE_URL}" -o "${ROOT}/compose.yaml"
fi
umask 077
printf 'BEACON_IMAGE_REF=%s\n' "${IMAGE_REF}" > "${ROOT}/.env"
chmod 0600 "${ROOT}/.env"

if [[ ! -e ${CONFIG_DIR}/policy.json ]]; then
  printf '{"rules":[]}\n' > "${CONFIG_DIR}/policy.json"
  chown root:10001 "${CONFIG_DIR}/policy.json"
  chmod 0640 "${CONFIG_DIR}/policy.json"
fi

docker compose --env-file "${ROOT}/.env" -f "${ROOT}/compose.yaml" config --quiet
echo "installed Beacon server layout in ${ROOT}; Compose configuration is valid"
