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
- Agent credentials are individual, revocable, and rotatable from the server
  credentials directory without putting secrets in the repository.
- Enrollment codes are single-use, expiring, stored only as hashes by the server,
  and must be transferred and deleted as bootstrap secrets.
- The server binds only to an explicitly configured interface.
- Channel credentials are stored only on the server through a secret mechanism.
- Logs contain event identifiers and delivery status, not raw payloads.
- Transport credentials are read from files rather than command-line arguments.
- Telegram tokens are read from a separate restricted file; Telegram config
  contains only the token-file path and chat identifier.
- Telegram delivery uses HTTPS, a bounded timeout, and never logs the bot token.
- The bootstrap uses per-agent bearer files and requires HTTPS by default.
- Production transport uses server certificate/key files and CA validation by
  agents; HTTP is only a local-development escape hatch.
- Certificate issuance, renewal, permissions, and private-key custody belong
  to the deployment infrastructure, not the Beacon process.

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
