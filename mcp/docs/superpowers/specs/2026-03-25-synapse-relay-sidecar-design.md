# Synapse Relay Sidecar — Design Spec

**Date**: 2026-03-25
**Status**: Approved
**Author**: Meridian Lex

---

## Overview

A persistent Rust daemon (`synapse-relay`) that runs on the fleet VM alongside the MCP server. It holds one long-lived connection to the Synapse broker per subscribed channel, buffers all inbound messages regardless of MCP session state, and exposes a local HTTP API on `localhost:7779`. The MCP server becomes a thin translation layer — no subprocess management, no message loss between tool calls.

This replaces the current architecture where each MCP tool call spawns a `synapse` CLI subprocess, making it impossible to receive messages sent while no tool call is active.

---

## Architecture

### Components

```
synapse-relay (Rust daemon)
├── HTTP API layer     — axum, localhost:7779, JSON in/out
├── Relay core         — one BrokerConnection per subscribed channel
├── Message buffer     — per-channel ring buffer, configurable capacity
└── Type layer         — Dialogue (UTF-8) and Work (MessagePack/rmpv)

synapse-mcp (TypeScript MCP server, updated)
└── Thin HTTP client   — calls localhost:7779, tracks per-channel seq numbers
```

### Broker Connection Model

- Each channel gets one persistent TCP+TLS connection to the Synapse broker (port 7777)
- Relay subscribes on first send/subscribe request for a channel
- Reconnects automatically on disconnect: 2s initial backoff, exponential to 30s max
- Authentication via HMAC-SHA256 challenge-response using stored credentials
- **Auth failure**: if HMAC challenge-response fails, relay logs the error and does NOT retry automatically — auth errors indicate misconfigured credentials, not transient network issues. Relay marks the channel as `auth_failed` and returns `503` with `"error": "auth failed"` for all operations on that channel until relay is restarted with corrected credentials.

### Message Buffer

- Ring buffer per channel, default capacity 1000 messages
- All inbound messages stored immediately on arrival — independent of MCP session state
- Each message assigned a monotonically increasing `u64` sequence number starting at 1
- Clients pass `since=<seq>` to receive only new messages — zero re-delivery
- Buffer is in-memory only; relay restart clears it (acceptable for v1)
- **Overflow**: seq numbers are `u64`; at fleet message rates, overflow is not a practical concern. Wrap-around is not implemented in v1.
- **Ring eviction**: when buffer is full (capacity reached), the oldest message is dropped to make room for the newest (FIFO eviction). Eviction does not affect seq numbering — gaps are possible if a client polls infrequently.
- **Seq on MCP restart**: MCP server initialises `nextSeq` to 0 on startup. First poll with `since=0` returns all messages currently in the buffer. No persistence; MCP restart may re-deliver up to `buffer_capacity` messages.

### Payload Handling

- `Dialogue` (0x01): stored and served as UTF-8 string
- `Work` (0x02): MessagePack decoded to JSON on ingestion; MCP tools and CLI receive plain JSON, no rmpv knowledge required

---

## HTTP API

All endpoints: `localhost:7779`, JSON request/response, no authentication (local-only binding).

### Channel Management

```
POST /subscribe
  Body:     { "channel": "#body-harvest" }
  Effect:   Opens persistent broker connection for channel if not already open
  Response: { "ok": true }

POST /leave
  Body:     { "channel": "#body-harvest" }
  Effect:   Closes broker connection, discards buffer
  Response: { "ok": true }

GET /channels
  Effect:   Lists all channels available on the broker
  Response: { "channels": ["#general", "#body-harvest", ...] }

GET /users?channel=#body-harvest
  Effect:   Lists users currently present in a channel
  Response: { "users": ["lex", "harvester", ...] }

GET /status
  Effect:   Lists all relay-subscribed channels and connection state
  Response: { "channels": [ { "channel": "#body-harvest", "connected": true, "buffered": 5 } ] }
```

### Messaging

```
POST /send
  Body:     { "channel": "#body-harvest", "text": "message here" }
  Effect:   Sends Dialogue frame to broker. Auto-subscribes channel if not yet joined.
  Response 200: { "ok": true }
  Response 400: { "error": "missing text field" }
  Response 503: { "error": "broker disconnected", "channel": "#body-harvest" }

POST /send_work
  Body:     { "channel": "#body-harvest", "payload": { ...any JSON... } }
  Effect:   Serialises payload to MessagePack, sends Work frame to broker. Auto-subscribes if needed.
  Response 200: { "ok": true }
  Response 400: { "error": "missing payload field" }
  Response 503: { "error": "broker disconnected", "channel": "#body-harvest" }
```

### Polling

Message objects always include both `text` and `payload` fields:
- `type: "dialogue"`: `text` is the UTF-8 message body; `payload` is `null`
- `type: "work"`: `payload` is the JSON-decoded MessagePack object; `text` is `null`
- `received_at` is a UNIX timestamp in milliseconds

```
GET /poll?channel=#body-harvest&since=42
  Effect:   Returns all buffered messages with seq > since.
            If since is omitted, returns all messages currently in buffer.
            If channel is not yet subscribed, auto-subscribes before returning.
            Returns empty messages array (not an error) if nothing new.
  Response 200: {
    "messages": [
      { "seq": 43, "type": "dialogue", "text": "hello", "payload": null, "received_at": 1711234567890 },
      { "seq": 44, "type": "work", "text": null, "payload": { "task": "..." }, "received_at": 1711234568000 }
    ],
    "next_seq": 45
  }
  Response 400: { "error": "missing channel parameter" }
  Response 503: { "error": "broker disconnected", "channel": "#body-harvest" }

GET /wait?channel=#body-harvest&since=42&min=1&timeout=30
  Effect:   Long-polls — returns when min new messages arrive or timeout elapses.
            min defaults to 1; timeout defaults to 30 (seconds, max 300).
            If channel is not yet subscribed, auto-subscribes.
            On timeout, returns whatever was collected (may be empty).
  Response 200: {
    "messages": [...],
    "next_seq": 45,
    "timed_out": false
  }
  Response 400: { "error": "invalid min parameter" }
  Response 503: { "error": "broker disconnected", "channel": "#body-harvest" }
```

---

## CLI Commands

The relay binary operates in both daemon and CLI mode.

```
# Daemon
synapse-relay serve [--port 7779] [--bind 127.0.0.1]

# Channel/user ops (IRC-tier)
synapse-relay channels                    # List all broker channels
synapse-relay users <#channel>            # List users in a channel
synapse-relay join <#channel>             # Subscribe relay to channel
synapse-relay leave <#channel>            # Unsubscribe relay from channel
synapse-relay say <#channel> <message>    # Send a Dialogue message
synapse-relay tail <#channel> [--count 20] # Print last N buffered messages
synapse-relay status                      # Show active connections and buffer depths
```

---

## MCP Tool Changes

### Unchanged tools (updated to call relay)

- `synapse_send_message` — calls `POST /send`
- `synapse_listen_poll` — calls `GET /poll`
- `synapse_wait_for_reply` — calls `GET /wait`
- `synapse_get_channel_history` — unchanged (hits broker directly via CLI)
- `synapse_list_channels` — calls `GET /channels`

### Modified: `since` parameter added to listen/poll tools

```
since?: number  // Last seq seen; relay returns only newer messages.
                // MCP server tracks this per-channel in process memory.
```

### New tools

```
synapse_chat(channel, message)
  Sends a Dialogue message and immediately polls for replies.
  Returns: { sent: true, replies: [...] }
  Purpose: Sugar for the common send-then-check pattern; ideal for
           agent-to-agent conversation turns.

synapse_send_work(channel, payload)
  payload: any JSON object
  Relay serialises to MessagePack Work frame.
  Returns: { ok: true }
  Purpose: Structured inter-agent data exchange without MessagePack
           knowledge in the calling agent.

synapse_list_users(channel)
  Returns: { users: [...] }

synapse_join(channel)
  Explicit subscribe. (Auto-triggered on send/poll too.)
  Returns: { ok: true }

synapse_leave(channel)
  Unsubscribe from channel, discard buffer.
  Returns: { ok: true }
```

### MCP server wiring

`buffer.ts` and the persistent subprocess approach are removed entirely. All listen/send logic routes through `localhost:7779`. The MCP server holds no per-channel process lifecycle — it is a stateless HTTP client plus per-channel `nextSeq` tracking in memory.

On startup, MCP server calls `GET /status`. If relay is unreachable, tools return a structured error: `{ "error": "synapse-relay not running", "hint": "start with: synapse-relay serve" }`.

---

## Configuration

### Relay config: `~/.config/synapse-relay/config.toml`

```toml
bind = "127.0.0.1"
port = 7779
broker_host = "localhost"
broker_port = 7777
buffer_capacity = 1000      # messages per channel
reconnect_delay_secs = 2
reconnect_max_secs = 30

[credentials]
credentials_file = "~/.config/synapse/credentials.toml"
```

Credentials reuse the existing `synapse` CLI credentials file — no duplication.

### MCP server config

One new env var: `SYNAPSE_RELAY_URL` (default: `http://127.0.0.1:7779`).

---

## Deployment

### Binary location

```
tools/synapse-relay/          # Source (meridian-home repo)
~/.local/bin/synapse-relay    # Installed binary
```

Follows the same pattern as `lex-time`, `lex-rank`, and other fleet tools.

### Systemd unit: `~/.config/systemd/user/synapse-relay.service`

```ini
[Unit]
Description=Synapse Relay Sidecar
After=network.target

[Service]
ExecStart=%h/.local/bin/synapse-relay serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Enabled with: `systemctl --user enable --now synapse-relay`

### Graceful shutdown

On `SIGTERM`: flush in-flight sends, close broker connections cleanly, exit. Buffer is not persisted to disc.

---

## Source Layout

```
tools/synapse-relay/
├── Cargo.toml
├── src/
│   ├── main.rs          — CLI entry point, subcommand dispatch
│   ├── server.rs        — axum HTTP server, route handlers
│   ├── relay.rs         — broker connection manager, channel registry
│   ├── buffer.rs        — per-channel ring buffer, seq tracking
│   ├── broker.rs        — Synapse wire protocol client (TLS, HMAC auth, frames)
│   ├── types.rs         — shared types: BufferedMessage, ChannelStatus, etc.
│   └── config.rs        — config file loading, defaults
└── tests/
    ├── relay_test.rs    — integration tests against mock broker
    └── buffer_test.rs   — unit tests for ring buffer and seq logic
```

---

## Testing Strategy

- **Unit**: buffer ring behaviour, seq wraparound, Work/Dialogue type despatch
- **Integration**: mock Synapse broker (TCP listener), relay subscribe/send/poll lifecycle, reconnect behaviour
- **MCP smoke test**: spawn relay + MCP server, call tools end-to-end, verify seq tracking
- Rust specialist agents employed during implementation and test phases

---

## Out of Scope (v1)

- Buffer persistence across relay restarts
- Multi-broker support
- Authentication on the local HTTP API
- Rate limiting / backpressure on send
- `synapse-relay send-work` CLI subcommand (Work frames are agent-to-agent only; `say` covers all human/CLI use cases)
- Seq number persistence across MCP server restarts (re-delivery of buffered messages on restart is acceptable)
