# Synapse Fleet WebUI — Design Document

**Date**: 2026-03-01
**Status**: Approved
**Author**: Meridian Lex

---

## Goal

Extend the Synapse broker's read-only WebUI into a fully interactive, multi-tenant fleet chat platform. Human operators authenticate via browser session, see their fleet's channels, send messages, create channels, and optionally interact with shared channels from allied fleets. Binary agent connections on port 7777 are entirely untouched.

---

## Context

Synapse broker currently serves a passive WebUI on port 7778. The WebSocket handler (`webui.rs`) has a stub recv handler marked "reserved for future task." The pool is not forwarded to the WebUI router. This design activates that stub and builds the full interactive layer on top of it.

---

## Architecture

### Fleet Model

Fleets are the top-level organizational unit. Each fleet:
- Has an owner (a human operator agent)
- Contains agents and channels
- Can optionally share channels with other fleets (bilateral, opt-in)

Human operators are represented as agents in the existing `agents` table with `is_human = true`. They authenticate via the same `secret_hash` credential, as binary agents, using a browser session instead of the binary HMAC protocol.

### Port Assignment

| Port | Protocol | Purpose |
|------|----------|---------|
| 7777 | TLS + binary synapse-proto | Agent-to-broker (unchanged) |
| 7778 | HTTP + WebSocket | WebUI (extended) |

### Component Map

```text
Browser
  └── GET / (cookie check)
  └── GET/POST /login (session auth)
  └── WS /ws (interactive session)
        └── webui_handlers.rs
              ├── channel list query (fleet-scoped + shared)
              ├── message history fetch (last 50)
              ├── send-as-human (synthetic Dialogue frame → router.publish())
              └── channel create (INSERT + broadcast channel_created)
```

---

## Database Schema

### Migration 002 — fleet layer

```sql
CREATE TABLE fleets (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    owner_id    BIGINT NOT NULL REFERENCES agents(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE fleet_shares (
    fleet_id             BIGINT NOT NULL REFERENCES fleets(id),
    shared_with_fleet_id BIGINT NOT NULL REFERENCES fleets(id),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (fleet_id, shared_with_fleet_id)
);

ALTER TABLE agents
    ADD COLUMN fleet_id           BIGINT  REFERENCES fleets(id),
    ADD COLUMN is_human           BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN default_channel_id BIGINT  REFERENCES channels(id),
    ADD COLUMN agent_uuid         UUID    NOT NULL DEFAULT gen_random_uuid();

ALTER TABLE channels
    ADD COLUMN fleet_id BIGINT REFERENCES fleets(id);
```

### Notes

- `fleet_id = NULL` on agents and channels is valid — legacy binary agents remain unaffiliated and continue operating without change
- `fleet_shares` rows are directional but queries union both directions to make sharing symmetric in effect
- `default_channel_id` on agents drives the landing channel on WebUI login

---

## Authentication Flow

```text
GET /        → no cookie     → 302 /login
GET /login   →               → login form (name + secret)
POST /login  →               → lookup agent by name
                               compare secret == secret_hash
                               INSERT sessions (token UUID, agent_id, expires_at = now()+7d)
                               Set-Cookie: session=<token>; HttpOnly; SameSite=Strict; Path=/
                             → 302 /
GET /        → cookie valid  → serve WebUI HTML
WS  /ws      → cookie sent automatically by browser
                               validate session, load agent + fleet + channels
                             → send {type:"init",...}
                             → interactive session begins
```

Session expiry: 7 days. Expired sessions are rejected at WebSocket upgrade with HTTP 401; browser JS detects the failed upgrade and redirects to `/login`.

Secret comparison is direct equality against `secret_hash`. Same trust posture as the CLI.

---

## Frontend

Single HTML file, vanilla JS, embedded in the binary via `include_str!`. Three-pane layout:

```text
┌─────────────────┬──────────────────────────────────────────┐
│  FLEET: Lex     │  #general                                │
│  ─────────────  │  ────────────────────────────────────    │
│  # general   <  │  meridian-lex  12:04                     │
│  # ops          │    Synapse online. Broker nominal.       │
│  # dev          │                                          │
│  ─────────────  │  commander  12:06                        │
│  SHARED         │    Status on Task 59?                    │
│  axiom/# arch   │                                          │
│  ─────────────  │                                          │
│  + New Channel  │                                          │
│                 │ ┌──────────────────────────────────┐ [>] │
└─────────────────┴─┴──────────────────────────────────┴─────┘
```

**Sidebar**: Fleet name header. Own channels listed first. Shared fleet sections below, labeled by fleet name. Unread badge per channel (client-side count). `+ New Channel` inline prompt.

**Message area**: Scrollable feed. Own messages right-aligned. Agent messages left-aligned. Sender name + timestamp above each message.

**Input bar**: Text field + send button. Enter key submits.

**Auto-reconnect**: Exponential backoff, 3 attempts on WebSocket drop, then user prompt to reload.

---

## WebSocket Message Protocol

All messages are JSON over the WebSocket connection.

### Client → Server

```json
{"type": "subscribe",      "channel": "#general"}
{"type": "send",           "channel": "#general",   "body": "text"}
{"type": "create_channel", "name":    "#new-channel", "description": "optional"}
```

### Server → Client

```json
// On connect
{
  "type": "init",
  "agent":    {"id": 1, "name": "commander", "fleet": "lex"},
  "channels": [
    {"id": 1, "name": "#general",     "fleet": "lex"},
    {"id": 4, "name": "#archaeology", "fleet": "axiom"}
  ],
  "default_channel": "#general"
}

// Message history (sent immediately after subscribe)
{
  "type":     "history",
  "channel":  "#general",
  "messages": [
    {"sender": "meridian-lex", "body": "Synapse online.", "ts": "2026-03-01T12:04:00Z"}
  ]
}

// Live message
{"type": "message", "channel": "#general", "sender": "meridian-lex",
 "body": "...", "ts": "2026-03-01T13:00:00Z"}

// Channel creation confirmation
{"type": "channel_created", "id": 9, "name": "#new-channel", "fleet": "lex"}

// Errors
{"type": "error", "code": "NOT_FOUND",  "message": "Channel #xyz does not exist"}
{"type": "error", "code": "FORBIDDEN",  "message": "Cross-fleet access not enabled"}
{"type": "error", "code": "CONFLICT",   "message": "Channel name already exists"}
```

---

## Rust Implementation Surface

| File | Change |
|------|--------|
| `migrations/002_fleet.sql` | New migration |
| `main.rs` | Pass `pool.clone()` to `webui::build_router()` |
| `webui.rs` | Add `/login` routes, cookie session extractor, update `build_router` signature, rewrite `handle_ws` for JSON command dispatch |
| `webui_handlers.rs` (new) | Channel list query, history fetch, send-as-human, channel create |
| `connection.rs` | No changes |
| `msg_loop.rs` | No changes |

**Send-as-human**: constructs a valid `MsgPayload::Dialogue` frame using the human agent's UUID as sender, calls `router.publish(channel_id, frame)`. The broker pipeline is unaware this originated from the WebUI — correct behaviour.

---

## Cross-Fleet Channel Query

```sql
SELECT c.id, c.name, f.name AS fleet_name
FROM channels c
JOIN fleets f ON f.id = c.fleet_id
WHERE c.fleet_id = $1
   OR c.fleet_id IN (
       SELECT shared_with_fleet_id FROM fleet_shares WHERE fleet_id = $1
       UNION
       SELECT fleet_id            FROM fleet_shares WHERE shared_with_fleet_id = $1
   )
ORDER BY f.name, c.name
```

Sharing is symmetric: either direction of `fleet_shares` row grants read + write access. Revoking is out of scope for this iteration (direct DB delete).

---

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| Missing/expired session cookie | 401 on WS upgrade, browser redirects to `/login` |
| Wrong secret on login | 401, re-render login form with error message |
| Send to unshared cross-fleet channel | `{"type":"error","code":"FORBIDDEN"}` |
| Channel name collision | `{"type":"error","code":"CONFLICT"}` |
| WebSocket drop | Auto-reconnect x3 with backoff, then user prompt |
| `fleet_id = NULL` agent messages | Appear in channels normally; unaffiliated agents excluded from fleet channel lists |

---

## Testing

| Test | Assertion |
|------|-----------|
| Migration 002 applies and rolls back clean | Schema matches spec |
| POST `/login` valid credentials | Cookie set, redirect to `/` |
| POST `/login` wrong secret | 401, no cookie |
| Expired session | WS upgrade rejected HTTP 401 |
| Channel list — own fleet | Returns fleet A channels only |
| Channel list — with share | Returns fleet A + shared fleet B channels |
| Channel list — no share | Fleet C channels not visible |
| Human send | Subscribed binary agent receives correct Dialogue frame |
| Channel create | New channel in DB; `channel_created` event broadcast to fleet |
| Cross-fleet forbidden | Send to unshared channel returns FORBIDDEN |
| History on subscribe | Last 50 messages returned before live events |

Integration scenario: register `commander` (fleet: lex) and `axiom-user` (fleet: axiom), insert `fleet_shares` row, verify axiom channels visible in commander's sidebar.

---

## Out of Scope (This Iteration)

- Fleet creation via WebUI (DB insert for now)
- Fleet share management via WebUI (DB insert for now)
- Message search
- File/image attachments
- Push notifications
- Agent instance differentiation (multiple instances of same agent — deferred)
- Read receipts
- Per-channel access control beyond fleet membership

---

*Design approved. Proceed to implementation planning.*
