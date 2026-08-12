# Homelab Homologation And Pilot

This runbook is a proposal for the Beacon pilot on Ops (`192.168.68.62`) and
Media (`192.168.68.60`). It is not authorization to change either host. Before
execution, confirm live state over SSH, read `docs/10-operacao.md` and the
applicable service runbooks, and obtain explicit authorization for every
privileged or Docker operation.

The operator-facing setup is summarized in `docs/quickstart.md`; this document
is the homologation checklist, evidence plan, and rollback procedure.

## Scope

- Deploy `beacon-server` in the existing Ops Docker host.
- Enroll one native `beacon-agent` on Media.
- Use the internal HTTP pilot only with `--allow-http`.
- Validate event intake, durable spool behavior, alert lifecycle, Telegram
  notification, restart recovery, rotation, revocation, and rollback.
- Do not onboard production detectors or personal-data workflows during the
  pilot.

## Release

Use only the immutable image reference:

```text
ghcr.io/rogeriosobrinho/beacon-alerts@sha256:3ab439d5497d69852aaf41594e84dbede1437aea388a5d60c456d97000c3ea20
```

Release: `v0.1.0-rc3`

## Preconditions

- Confirm Ops and Media identities, current services, mounts, free space, and
  firewall rules via approved live inspection.
- Confirm Docker Compose is available on Ops and the `ops` account has no raw
  Docker socket access; use the approved admin path for Docker operations.
- Confirm `/var/lib/beacon/events`, `/etc/beacon/agents.d`, and the temporary
  enrollment directory have an approved owner, group, and mode.
- Prepare a narrow policy rule matching only the pilot event.
- Prepare Telegram configuration and token through the secret-management path;
  never print or commit secrets.
- Confirm the Ops-to-Telegram egress path and Media-to-Ops TCP `8787` path.
- Record the current image reference and configuration for rollback.
- Confirm an approved backup or snapshot procedure for the Beacon data directory.

## Phase 0: Offline Validation

Run without touching the homelab:

```sh
cargo fmt --all -- --check
cargo check --locked --bin beacon-agent --no-default-features
cargo check --locked --bin beacon-server --features server
cargo clippy --locked --bin beacon-agent --no-default-features -- -D warnings
cargo clippy --locked --bin beacon-server --features server -- -D warnings
cargo test --locked --no-default-features
cargo test --locked --features server
docker compose --env-file .env.example config --quiet
```

Expected result: all commands succeed and the image reference is the rc3
digest above.

## Phase 1: Ops Deployment

1. Create or verify the approved directories and restricted configuration files.
2. Set `BEACON_IMAGE_REF` to the rc3 digest.
3. Run `docker compose --env-file .env config --quiet`.
4. Pull the immutable image and record the resolved image digest.
5. Start the stack once, then inspect container status, logs, UID `10001`,
   mounts, port `8787`, and the notification worker.
6. Verify `GET /healthz` from an approved network location.
7. Confirm no unexpected listener, restart loop, failed mount, or notification
   worker error.

Rollback: stop the Compose project, restore the prior image reference, validate
Compose configuration, and restart. Keep the SQLite data directory intact.

## Phase 2: Agent Enrollment

1. From the approved Ops administration path, create a Media enrollment code
   with a short TTL, for example 15 minutes.
2. Transfer only the code file to Media through the approved secret-transfer
   process. Do not paste the code into chat or shell history.
3. Run `beacon-agent enroll` on Media with the internal HTTP pilot URL and
   `--allow-http`.
4. Verify the token file exists only on Media with mode `0600`; verify the code
   file is removed after successful enrollment.
5. Verify the server created exactly one credential file and did not log the
   code or token.
6. Attempt code reuse and an expired code in a disposable enrollment; both must
   fail without creating a credential.

## Phase 3: Functional Pilot

Run a single controlled event from Media:

```sh
beacon-agent send \
  --event-type beacon.test \
  --source manual \
  --host media \
  --state firing \
  --severity warning \
  --fingerprint beacon/test/media \
  --facts '{"message":"pilot firing"}' \
  --spool /var/lib/beacon/spool
beacon-agent run \
  --server http://192.168.68.62:8787 \
  --allow-http \
  --spool /var/lib/beacon/spool \
  --token-file /etc/beacon/agent.token
```

Verify:

- the event is accepted with `202`;
- the local spool removes the event only after acknowledgement;
- SQLite contains the event and one firing alert;
- exactly one Telegram firing notification arrives;
- a matching resolved event produces the expected resolved state and message;
- replaying the same event ID is idempotent;
- a conflicting payload for the same event ID returns `409`;
- unauthorized and malformed requests are rejected.

## Phase 4: Failure And Recovery

- Stop the server, queue an event on Media, run the agent, and verify the spool
  retains the event without an acknowledgement.
- Restart the server and verify the event drains once without duplication.
- Stop Telegram delivery or use a controlled failure path and verify durable
  notification retry state, bounded attempts, and eventual recovery.
- Restart the server during pending notification work and verify stale jobs are
  reclaimable.
- Rotate the Media credential atomically and verify the old token is rejected
  and the new token is accepted without server restart.
- Remove the Media credential and verify delivery is rejected.

## Observability And Evidence

Record only metadata and redacted output:

- release, image digest, commit, and deployment timestamp;
- Compose config validation and container status;
- listener and firewall verification;
- enrollment success/failure status without codes or tokens;
- event IDs, fingerprints, response statuses, and alert counts;
- notification job states and Telegram delivery timestamps;
- restart, rollback, rotation, and revocation results;
- journald and journal-upload observations;
- SMART/storage checks on the PVE host when the workflow is accepted for a
  longer pilot.

## Exit Criteria

The pilot is accepted only if all functional, security, recovery, and rollback
checks pass; no secret appears in logs or evidence; the agent spool is durable;
the notification queue is durable; and the operator can return to the previous
image without data loss. A failed criterion blocks production detector
onboarding and requires a defect record or explicit waiver.

## Stop Conditions

Stop immediately for unexpected network exposure, an unapproved mount change,
secret exposure, data loss, duplicate notifications that cannot be explained,
credential acceptance after revocation, failed rollback, or any host state that
differs from the approved precondition record.
