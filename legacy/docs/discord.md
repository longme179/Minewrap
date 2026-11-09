# Discord notifier

Every instance can push its state plus the most recent console lines to a
Discord webhook. The push runs **inside the daemon**, so it keeps working when
the GUI is closed. It is configured per instance in `config.toml`:

```toml
[instances.discord]
enabled = true
webhook_url = "https://discord.com/api/webhooks/<id>/<token>"  # secret
mode = "edit-in-place"    # default; or "new-message"
interval_seconds = 600    # update cadence (default 10 minutes)
first_push_delay_seconds = 10
log_lines = 15            # tail lines included in the embed
max_log_chars = 1500      # hard cap for the log block (Discord limit: 4096)
state_change_posts = true
state_change_debounce_seconds = 30
redact_regexes = []       # extra patterns, e.g. ["token=\\S+"]
```

The webhook URL is a secret: it is never logged, never returned by the API
(`webhook_set: true` is all the GUI sees), and files written by the daemon are
mode `0600`.

## Modes

- **edit-in-place** (default): the first push creates a message with
  `POST <webhook>?wait=true` and stores the message id in
  `<data_dir>/discord/<instance-id>.json`. Every interval then updates it with
  `PATCH <webhook>/messages/<message-id>`. If the message was deleted (HTTP
  404), a new message is created and the state file is updated. One message per
  instance, no channel spam.
- **new-message**: a fresh message every interval. Useful for channels where
  you want a history of snapshots.

## What the embed contains

- Title: `<name> — <combined state>` with a colour per state (green online,
  yellow starting, orange stopping, grey offline, red crashed, dark red
  unresponsive).
- Fields: version, players online/max (+ player names when Query is enabled),
  latency, uptime, last-updated (Discord relative timestamp).
- A fenced code block with the last `log_lines` console lines (default 15),
  capped at `max_log_chars` characters. When over budget, **oldest lines are
  dropped first**; a single over-long line is truncated.

Discord limits are respected: embed title ≤ 256, description ≤ 4096, field
value ≤ 1024, total embed ≤ 6000. Defaults stay far below (≈1500 chars of logs).

## Log hygiene (before anything leaves the machine)

1. ANSI colour codes are stripped.
2. The Minecraft login form `Alice[/1.2.3.4:5678]` becomes
   `Alice[/redacted]`.
3. Bare IPv4 and IPv6 addresses become `[redacted]`.
4. Each `redact_regexes` pattern is replaced with `[redacted]`.
5. Lines are trimmed to 300 characters.

Known limitation: exotic IPv6 spellings may slip through the built-in
patterns — add a `redact_regexes` entry if you need stricter filtering.

## Immediate state-change posts

When the instance state changes (started, stopped, crashed, restart loop), an
extra compact message is posted right away instead of waiting for the next
interval. Posts are debounced by `state_change_debounce_seconds` (default 30 s)
so a crash loop cannot spam the channel: changes inside the window are
coalesced into one post when the window ends. Set
`state_change_posts = false` to disable.

## Reliability

- Requests use a 10 s timeout.
- HTTP 429: the daemon sleeps for Discord's `retry_after` (seconds, capped at
  60 s) and retries, up to 3 attempts.
- HTTP 5xx: exponential backoff (1 s, 2 s, 4 s … capped at 60 s), up to 3
  attempts.
- Network errors: one warning per push attempt, then the interval is skipped.
- A failed push never crashes the notifier loop, the daemon, or the server.
- Every failure logs exactly one clear line with the webhook URL masked:
  `https://discord.com/api/webhooks/***/***`.

## Testing the webhook

From the GUI's Settings tab, or directly:

```sh
curl -X POST -H "Authorization: Bearer <token>" \
  http://127.0.0.1:8080/api/v1/instances/<id>/discord/test
# {"ok": true, "error": null}
```

If Discord is disabled or no webhook is configured, the endpoint answers
`{"ok": false, "error": "discord is disabled or no webhook is configured"}`.

## Creating a webhook

In Discord: Server Settings → Integrations → Webhooks → New Webhook → pick the
channel → Copy Webhook URL. Treat it like a password: anyone with the URL can
post to that channel.
