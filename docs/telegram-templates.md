# Telegram Message Templates

Beacon keeps Telegram presentation outside the binary. Configure it in the
server's `/etc/beacon/telegram.json`; start from
[`telegram.example.json`](telegram.example.json). The bot token remains in the
separate restricted `token_file` and must never be placed in this configuration.

## Configuration Shape

`formats` defines the global layout for `firing`, `resolved`, and `info`. The
`events` map overrides the title, state text, and safe fields for one event type.

## Safe Placeholders

- `{icon}`, `{status}`, `{host}`, `{title}`, `{detail}`, `{action}`;
- `{severity}`;
- `{fields}`, containing only explicitly mapped scalar facts.

Unknown placeholders are rejected at startup. Fingerprints, raw event types,
raw facts, paths, tokens, keys, cookies, personal data, and arbitrary payloads
are not direct placeholders.

## Dynamic Fields

Map fields explicitly:

```json
"fields": [
  {"label": "Duration", "fact": "duration_seconds"},
  {"label": "Files", "fact": "files"}
]
```

Only string, number, and boolean values are rendered. Values are bounded and
control characters are removed. Detectors must keep facts allowlisted and free
of secrets.

## Formatting And Persistence

Messages are plain text by default. Telegram Markdown/HTML is intentionally not
enabled implicitly. Beacon renders the message when it accepts the event and
creates the durable notification job; retries resend the same payload. Restart
Beacon after changing templates to apply them to future events.
