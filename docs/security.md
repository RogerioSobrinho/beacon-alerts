# Security Model

Beacon is intended to receive operational facts without becoming a remote
administration plane.

## Rules

- Agents never receive Telegram credentials.
- The server never executes commands on agents.
- Agents do not need Docker socket access.
- Payload size and field names are strictly limited.
- Event facts are allowlisted by event type.
- Secrets, tokens, cookies, keys, personal data, and backup contents are never
  valid event facts.
- Agent credentials are individual, revocable, and rotatable.
- The server binds only to an explicitly configured interface.
- Channel credentials are stored only on the server through a secret mechanism.
- Logs contain event identifiers and delivery status, not raw payloads.
- Transport credentials are read from files rather than command-line arguments.
- The bootstrap uses a single bearer token and plain HTTP; it is intended only
  for a controlled trusted network until per-agent credentials and TLS exist.

## Threats to Test

- forged agent event;
- replayed event;
- duplicated delivery;
- oversized payload;
- unknown event type;
- secret in facts;
- server unavailable;
- Telegram unavailable;
- compromised agent attempting remote execution;
- notification storm.
