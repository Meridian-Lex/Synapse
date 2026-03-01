# Synapse Fleet WebUI Implementation Plan

> **For Lex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the Synapse broker's read-only WebUI into a fully interactive multi-tenant fleet chat platform with HTTP session auth, channel sidebar, message send, channel creation, and cross-fleet visibility.

**Architecture:** Human operators authenticate via HTTP form login (secret compared against `secret_hash`), receive a session cookie, then communicate over WebSocket using a JSON command protocol. Fleet ownership scopes channel visibility; bilateral `fleet_shares` rows enable cross-fleet access. Binary agent connections on port 7777 are entirely untouched.

**Tech Stack:** Rust + axum 0.7, sqlx 0.8 (postgres + chrono + uuid), vanilla JS (embedded via include_str!), PostgreSQL migrations

**Design doc:** `docs/plans/2026-03-01-fleet-webui-design.md`

---

## Working Branch

All work on `feat/fleet-webui`. Never commit to master directly.

```bash
git checkout feat/fleet-webui

```

---

### Task 1: Cargo.toml — add chrono, uuid, and sqlx uuid feature

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/synapse-broker/Cargo.toml`

**Step 1: Update `[workspace.dependencies]` in `Cargo.toml`**

Add:

```toml
chrono = { version = "0.4", features = ["serde"] }
uuid   = { version = "1",   features = ["v4"] }

```

Update the existing sqlx entry to add `"uuid"` to features:

```toml
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio", "chrono", "migrate", "uuid"] }

```

**Step 2: Add to `crates/synapse-broker/Cargo.toml` `[dependencies]`**

```toml
chrono = { workspace = true }
uuid   = { workspace = true }

```

**Step 3: Verify compilation**

```bash
cargo build -p synapse-broker 2>&1 | head -20

```

Expected: no errors (warnings OK at this stage)

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/synapse-broker/Cargo.toml
git commit -m "chore(broker): add chrono, uuid deps; add sqlx uuid feature"

```

---

### Task 2: Migration 002 — fleet schema

**Files:**
- Create: `migrations/002_fleet.sql`

**Step 1: Write the migration**

```sql
-- migrations/002_fleet.sql

CREATE TABLE fleets (
    id         BIGSERIAL PRIMARY KEY,
    name       TEXT    NOT NULL UNIQUE,
    owner_id   BIGINT  NOT NULL REFERENCES agents(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
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

**Step 2: Check if sqlx auto-migration is wired up**

```bash
grep -r "migrate" crates/synapse-broker/src/

```

If `db::connect` does not call `sqlx::migrate!()`, add this to `main.rs` after `db::connect`:

```rust
sqlx::migrate!("../../migrations").run(&pool).await?;

```

**Step 3: Apply the migration**

Restart the broker to trigger auto-migration:

```bash
cd /home/meridian/meridian-home/projects/Gantry
LEX_CERTS_DIR=/home/meridian/meridian-home/lex-internal/certs \
  docker compose -f communication/synapse/docker-compose.yml restart synapse-broker
docker logs stratavore-synapse 2>&1 | grep -i migrat

```

Expected output: `Applied 1 migration(s)` or similar.

**Step 4: Verify schema**

```bash
docker exec stratavore-postgres psql -U postgres -d synapse -c "\d fleets"
docker exec stratavore-postgres psql -U postgres -d synapse -c "\d agents" | grep -E "fleet|human|uuid|default"

```

**Step 5: Commit**

```bash
git add migrations/002_fleet.sql
git commit -m "feat(db): migration 002 — fleet, fleet_shares, agent_uuid columns"

```

---

### Task 3: WebUiState struct + update build_router signature

**Files:**
- Modify: `crates/synapse-broker/src/webui.rs`
- Modify: `crates/synapse-broker/src/main.rs`

**Step 1: Replace the top of `webui.rs`** with the new state struct and updated imports

```rust
use crate::router::Router;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Form, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router as AxumRouter,
};
use chrono::Utc;
use rand::Rng;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::warn;

#[derive(Clone)]
pub struct WebUiState {
    pub broker_router: Arc<Router>,
    pub pool: PgPool,
}

pub fn build_router(broker_router: Arc<Router>, pool: PgPool) -> AxumRouter {
    let state = WebUiState { broker_router, pool };
    AxumRouter::new()
        .route("/",      get(serve_index))
        .route("/login", get(serve_login).post(handle_login))
        .route("/ws",    get(ws_handler))
        .with_state(state)
}

```

**Step 2: Update `main.rs`** — pass pool to build_router

Find:

```rust
if let Err(e) = axum::serve(listener, webui::build_router(wr)).await {

```

Replace with:

```rust
if let Err(e) = axum::serve(listener, webui::build_router(wr, pool.clone())).await {

```

**Step 3: Compile check** (will fail on missing handler stubs — that is expected)

```bash
cargo build -p synapse-broker 2>&1 | grep "^error" | head -20

```

Expected: errors about missing `serve_index`, `serve_login`, `handle_login`, `ws_handler`. NOT signature errors on `build_router`. Fix any signature errors before proceeding.

---

### Task 4: Login page and GET /login route

**Files:**
- Create: `webui/login.html`
- Modify: `crates/synapse-broker/src/webui.rs`

**Step 1: Write `webui/login.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Synapse — Login</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: #0d1117; color: #e6edf3; font-family: monospace;
         display: flex; align-items: center; justify-content: center;
         min-height: 100vh; }
  .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px;
          padding: 2rem; width: 360px; }
  h1 { font-size: 1.1rem; margin-bottom: 1.5rem; color: #58a6ff;
       letter-spacing: 0.1em; }
  label { display: block; font-size: 0.8rem; color: #8b949e; margin-bottom: 0.3rem; }
  input { width: 100%; padding: 0.5rem 0.75rem; background: #0d1117;
          border: 1px solid #30363d; border-radius: 4px; color: #e6edf3;
          font-family: monospace; font-size: 0.9rem; margin-bottom: 1rem; }
  input:focus { outline: none; border-color: #58a6ff; }
  button { width: 100%; padding: 0.6rem; background: #238636;
           border: none; border-radius: 4px; color: #fff;
           font-family: monospace; font-size: 0.9rem; cursor: pointer; }
  button:hover { background: #2ea043; }
  .error { color: #f85149; font-size: 0.8rem; margin-bottom: 1rem; }
</style>
</head>
<body>
<div class="card">
  <h1>SYNAPSE // FLEET ACCESS</h1>
  {{ERROR}}
  <form method="post" action="/login">
    <label for="name">Agent Name</label>
    <input id="name" name="name" type="text" autocomplete="off" required>
    <label for="secret">Secret</label>
    <input id="secret" name="secret" type="password" required>
    <button type="submit">Authenticate</button>
  </form>
</div>
</body>
</html>

```

**Step 2: Add `serve_login` and `login_error_response` to `webui.rs`**

```rust
async fn serve_login() -> impl IntoResponse {
    Html(include_str!("../../../webui/login.html").replace("{{ERROR}}", ""))
}

fn login_error_response(msg: &str) -> Response {
    // Sanitize msg to prevent reflected XSS (msg is always a static string from our code)
    let safe_msg = msg.replace('<', "&lt;").replace('>', "&gt;");
    let body = include_str!("../../../webui/login.html")
        .replace("{{ERROR}}", &format!(r#"<p class="error">{safe_msg}</p>"#));
    (StatusCode::UNAUTHORIZED, Html(body)).into_response()
}

```

**Step 3: Compile check** — only `serve_index`, `handle_login`, `ws_handler` should now be missing

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

---

### Task 5: POST /login — session creation

**Files:**
- Modify: `crates/synapse-broker/src/webui.rs`

**Step 1: Add LoginForm and handle_login**

```rust
#[derive(Deserialize)]
struct LoginForm {
    name:   String,
    secret: String,
}

async fn handle_login(
    State(state): State<WebUiState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let agent = sqlx::query!(
        "SELECT id, secret_hash FROM agents WHERE name = $1 AND is_human = true",
        form.name
    )
    .fetch_optional(&state.pool)
    .await;

    let Ok(Some(agent)) = agent else {
        return login_error_response("Invalid agent name or secret.");
    };

    if agent.secret_hash != form.secret {
        return login_error_response("Invalid agent name or secret.");
    }

    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();

    let expires_at = Utc::now() + chrono::Duration::days(7);

    let insert = sqlx::query!(
        "INSERT INTO sessions (token, agent_id, expires_at) VALUES ($1, $2, $3)",
        token,
        agent.id,
        expires_at
    )
    .execute(&state.pool)
    .await;

    if insert.is_err() {
        return login_error_response("Internal error. Please try again.");
    }

    let cookie = format!(
        "session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800"
    );
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, "/"), (header::SET_COOKIE, cookie.as_str())],
        "",
    )
        .into_response()
}

```

**Step 2: Compile check**

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

Expected: only `serve_index` and `ws_handler` still missing.

---

### Task 6: GET / with session guard + placeholder index

**Files:**
- Modify: `crates/synapse-broker/src/webui.rs`
- Create: `webui/index.html` (placeholder — replaced in Task 13)

**Step 1: Add cookie extractor and session validator**

```rust
fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie_header
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.starts_with("session="))
        .map(|s| s["session=".len()..].to_string())
}

async fn validate_session(pool: &PgPool, token: &str) -> Option<i64> {
    let row = sqlx::query!(
        "SELECT agent_id FROM sessions WHERE token = $1 AND expires_at > now()",
        token
    )
    .fetch_optional(pool)
    .await
    .ok()??;
    Some(row.agent_id)
}

```

**Step 2: Add serve_index**

```rust
async fn serve_index(State(state): State<WebUiState>, headers: HeaderMap) -> Response {
    let token = match extract_session_token(&headers) {
        Some(t) => t,
        None => return Redirect::to("/login").into_response(),
    };
    if validate_session(&state.pool, &token).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    Html(include_str!("../../../webui/index.html")).into_response()
}

```

**Step 3: Create placeholder `webui/index.html`**

```html
<!DOCTYPE html>
<html><head><title>Synapse</title></head>
<body style="background:#0d1117;color:#e6edf3;font-family:monospace;padding:2rem">
<p>WebUI loading — implementation in progress.</p>
</body></html>

```

**Step 4: Compile check — should be zero errors now**

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

**Step 5: Commit**

```bash
git add crates/synapse-broker/src/webui.rs \
        crates/synapse-broker/src/main.rs \
        webui/login.html webui/index.html
git commit -m "feat(webui): HTTP session auth — login form, POST handler, session guard, cookie"

```

---

### Task 7: WebSocket handler — session validation and init message

**Files:**
- Create: `crates/synapse-broker/src/webui_handlers.rs`
- Modify: `crates/synapse-broker/src/webui.rs`
- Modify: `crates/synapse-broker/src/main.rs`

**Step 1: Create `webui_handlers.rs`**

```rust
// crates/synapse-broker/src/webui_handlers.rs

use serde::Serialize;
use sqlx::PgPool;
use synapse_proto::compression::decompress;
use synapse_proto::frame::{Encoding, FrameHeader, HEADER_LEN};

#[derive(Clone)]
pub struct AgentContext {
    pub agent_id:           i64,
    pub agent_name:         String,
    pub fleet_id:           Option<i64>,
    pub fleet_name:         Option<String>,
    pub default_channel_id: Option<i64>,
    pub agent_uuid:         uuid::Uuid,
}

#[derive(Serialize, Clone)]
pub struct ChannelInfo {
    pub id:         i64,
    pub name:       String,
    pub fleet_name: String,
}

/// Load full agent context from DB.
pub async fn load_agent_context(pool: &PgPool, agent_id: i64) -> Option<AgentContext> {
    let row = sqlx::query!(
        r#"
        SELECT a.id, a.name, a.fleet_id, a.default_channel_id,
               a.agent_uuid AS "agent_uuid: uuid::Uuid",
               f.name AS fleet_name
        FROM agents a
        LEFT JOIN fleets f ON f.id = a.fleet_id
        WHERE a.id = $1
        "#,
        agent_id
    )
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(AgentContext {
        agent_id:           row.id,
        agent_name:         row.name,
        fleet_id:           row.fleet_id,
        fleet_name:         row.fleet_name,
        default_channel_id: row.default_channel_id,
        agent_uuid:         row.agent_uuid,
    })
}

/// All channels visible to the given fleet (own + shared).
pub async fn fetch_channel_list(pool: &PgPool, fleet_id: i64) -> Vec<ChannelInfo> {
    sqlx::query_as!(
        ChannelInfo,
        r#"
        SELECT c.id, c.name, f.name AS fleet_name
        FROM channels c
        JOIN fleets f ON f.id = c.fleet_id
        WHERE c.fleet_id = $1
           OR c.fleet_id IN (
               SELECT shared_with_fleet_id FROM fleet_shares WHERE fleet_id = $1
               UNION
               SELECT fleet_id FROM fleet_shares WHERE shared_with_fleet_id = $1
           )
        ORDER BY f.name, c.name
        "#,
        fleet_id
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

#[derive(Serialize)]
pub struct MessageRecord {
    pub sender: String,
    pub body:   String,
    pub ts:     String,
}

/// Fetch last `limit` messages for a channel, returned oldest-first.
pub async fn fetch_history(pool: &PgPool, channel_id: i64, limit: i64) -> Vec<MessageRecord> {
    let rows = sqlx::query!(
        r#"
        SELECT a.name AS sender, m.body, m.created_at
        FROM (
            SELECT sender_id, body, created_at
            FROM messages
            WHERE channel_id = $1 AND content_type = 1
            ORDER BY created_at DESC
            LIMIT $2
        ) m
        JOIN agents a ON a.id = m.sender_id
        ORDER BY m.created_at ASC
        "#,
        channel_id,
        limit
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.into_iter()
        .map(|r| MessageRecord {
            sender: r.sender,
            body:   String::from_utf8_lossy(&r.body).to_string(),
            ts:     r.created_at.to_rfc3339(),
        })
        .collect()
}

/// Decode a raw broker frame to a displayable JSON string for the WebUI.
/// Returns None if the frame is not a Dialogue message.
pub fn frame_to_json(
    frame:             &[u8],
    active_channel_id: Option<i64>,
    channels:          &[ChannelInfo],
    sender_name:       &str,
) -> Option<String> {
    if frame.len() < HEADER_LEN {
        return None;
    }
    let header_bytes: &[u8; HEADER_LEN] = frame[..HEADER_LEN].try_into().ok()?;
    let hdr = FrameHeader::from_bytes(header_bytes).ok()?;

    let raw_payload = &frame[HEADER_LEN..];
    let payload: Vec<u8> = if hdr.encoding == Encoding::Zstd {
        decompress(raw_payload).ok()?
    } else {
        raw_payload.to_vec()
    };

    if payload.len() <= 17 || payload[0] != 0x01 {
        return None;
    }
    let body = std::str::from_utf8(&payload[17..]).ok()?;

    let channel_name = channels
        .iter()
        .find(|c| Some(c.id) == active_channel_id)
        .map(|c| c.name.as_str())
        .unwrap_or("#unknown");

    Some(serde_json::json!({
        "type":    "message",
        "channel": channel_name,
        "sender":  sender_name,
        "body":    body,
        "ts":      chrono::Utc::now().to_rfc3339(),
    }).to_string())
}

/// Create a channel in the DB scoped to the given fleet.
pub async fn create_channel(
    pool:        &PgPool,
    name:        &str,
    description: Option<&str>,
    fleet_id:    i64,
    creator_id:  i64,
) -> Result<ChannelInfo, anyhow::Error> {
    let row = sqlx::query!(
        r#"
        INSERT INTO channels (name, description, fleet_id, created_by)
        VALUES ($1, $2, $3, $4)
        RETURNING id, name
        "#,
        name,
        description,
        fleet_id,
        creator_id
    )
    .fetch_one(pool)
    .await?;

    let fleet_name = sqlx::query_scalar!(
        "SELECT name FROM fleets WHERE id = $1",
        fleet_id
    )
    .fetch_one(pool)
    .await?;

    Ok(ChannelInfo { id: row.id, name: row.name, fleet_name })
}

```

**Step 2: Add `mod webui_handlers;` to `main.rs`**

Add after existing `mod` declarations:

```rust
mod webui_handlers;

```

**Step 3: Add ws_handler and main WS loop to `webui.rs`**

```rust
async fn ws_handler(
    State(state): State<WebUiState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let token = match extract_session_token(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let agent_id = match validate_session(&state.pool, &token).await {
        Some(id) => id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let ctx = match crate::webui_handlers::load_agent_context(&state.pool, agent_id).await {
        Some(c) => c,
        None => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    ws.on_upgrade(move |socket| handle_ws_connection(socket, state, ctx))
}

async fn handle_ws_connection(
    mut socket: WebSocket,
    state:      WebUiState,
    ctx:        crate::webui_handlers::AgentContext,
) {
    let channels = match ctx.fleet_id {
        Some(fid) => crate::webui_handlers::fetch_channel_list(&state.pool, fid).await,
        None => vec![],
    };

    let default_channel = channels
        .iter()
        .find(|c| Some(c.id) == ctx.default_channel_id)
        .map(|c| c.name.clone());

    let init = serde_json::json!({
        "type": "init",
        "agent": {
            "id":    ctx.agent_id,
            "name":  ctx.agent_name,
            "fleet": ctx.fleet_name.as_deref().unwrap_or(""),
        },
        "channels":       channels,
        "default_channel": default_channel,
    });

    if socket.send(Message::Text(init.to_string())).await.is_err() {
        return;
    }

    let mut rx: Option<tokio::sync::broadcast::Receiver<Vec<u8>>> = None;
    let mut active_channel_id: Option<i64> = None;

    loop {
        if let Some(ref mut receiver) = rx {
            tokio::select! {
                result = receiver.recv() => {
                    match result {
                        Ok(frame) => {
                            if let Some(json) = crate::webui_handlers::frame_to_json(
                                &frame, active_channel_id, &channels, &ctx.agent_name,
                            ) {
                                if socket.send(Message::Text(json)).await.is_err() { break; }
                            }
                        }
                        Err(e) => {
                            warn!("webui: broadcast lag: {}", e);
                            break;
                        }
                    }
                }
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(m)) => {
                            if dispatch_command(m, &mut socket, &state, &ctx, &channels,
                                               &mut rx, &mut active_channel_id).await.is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            }
        } else {
            match socket.recv().await {
                Some(Ok(m)) => {
                    if dispatch_command(m, &mut socket, &state, &ctx, &channels,
                                       &mut rx, &mut active_channel_id).await.is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    }
}

```

**Step 4: Add dispatch_command stub**

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsCommand {
    Subscribe { channel: String },
    Send      { channel: String, body: String },
    CreateChannel { name: String, description: Option<String> },
}

async fn dispatch_command(
    msg:               Message,
    socket:            &mut WebSocket,
    state:             &WebUiState,
    ctx:               &crate::webui_handlers::AgentContext,
    channels:          &[crate::webui_handlers::ChannelInfo],
    rx:                &mut Option<tokio::sync::broadcast::Receiver<Vec<u8>>>,
    active_channel_id: &mut Option<i64>,
) -> Result<(), ()> {
    let text = match msg {
        Message::Text(t) => t,
        Message::Close(_) => return Err(()),
        _ => return Ok(()),
    };

    let cmd: WsCommand = match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(_) => {
            let err = serde_json::json!({"type":"error","code":"BAD_REQUEST",
                "message":"Invalid JSON command"});
            let _ = socket.send(Message::Text(err.to_string())).await;
            return Ok(());
        }
    };

    match cmd {
        WsCommand::Subscribe { channel } => {
            handle_subscribe(channel, socket, state, channels, rx, active_channel_id).await
        }
        WsCommand::Send { channel, body } => {
            handle_send(channel, body, socket, state, ctx, channels).await
        }
        WsCommand::CreateChannel { name, description } => {
            handle_create_channel(name, description, socket, state, ctx).await
        }
    }
}

async fn handle_subscribe(
    channel:           String,
    socket:            &mut WebSocket,
    state:             &WebUiState,
    channels:          &[crate::webui_handlers::ChannelInfo],
    rx:                &mut Option<tokio::sync::broadcast::Receiver<Vec<u8>>>,
    active_channel_id: &mut Option<i64>,
) -> Result<(), ()> {
    let ch = match channels.iter().find(|c| c.name == channel) {
        Some(c) => c,
        None => {
            let err = serde_json::json!({"type":"error","code":"NOT_FOUND",
                "message": format!("Channel {channel} not found or not accessible")});
            let _ = socket.send(Message::Text(err.to_string())).await;
            return Ok(());
        }
    };
    *rx = Some(state.broker_router.subscribe(ch.id).await);
    *active_channel_id = Some(ch.id);
    let history = crate::webui_handlers::fetch_history(&state.pool, ch.id, 50).await;
    let hist = serde_json::json!({"type":"history","channel":channel,"messages":history});
    let _ = socket.send(Message::Text(hist.to_string())).await;
    Ok(())
}

async fn handle_send(
    channel:  String,
    body:     String,
    socket:   &mut WebSocket,
    state:    &WebUiState,
    ctx:      &crate::webui_handlers::AgentContext,
    channels: &[crate::webui_handlers::ChannelInfo],
) -> Result<(), ()> {
    let ch = match channels.iter().find(|c| c.name == channel) {
        Some(c) => c,
        None => {
            let err = serde_json::json!({"type":"error","code":"NOT_FOUND",
                "message": format!("Channel {channel} not found or not accessible")});
            let _ = socket.send(Message::Text(err.to_string())).await;
            return Ok(());
        }
    };
    if let Err(e) = crate::webui_handlers::send_as_human(
        &state.pool, &state.broker_router, ctx, ch.id, &body,
    ).await {
        warn!("webui: send_as_human error: {}", e);
        let err = serde_json::json!({"type":"error","code":"INTERNAL","message":"Send failed"});
        let _ = socket.send(Message::Text(err.to_string())).await;
    }
    Ok(())
}

async fn handle_create_channel(
    name:        String,
    description: Option<String>,
    socket:      &mut WebSocket,
    state:       &WebUiState,
    ctx:         &crate::webui_handlers::AgentContext,
) -> Result<(), ()> {
    let Some(fleet_id) = ctx.fleet_id else {
        let err = serde_json::json!({"type":"error","code":"FORBIDDEN",
            "message":"No fleet assigned to your account"});
        let _ = socket.send(Message::Text(err.to_string())).await;
        return Ok(());
    };
    match crate::webui_handlers::create_channel(
        &state.pool, &name, description.as_deref(), fleet_id, ctx.agent_id,
    ).await {
        Ok(ch) => {
            let msg = serde_json::json!({"type":"channel_created",
                "id":ch.id,"name":ch.name,"fleet":ch.fleet_name});
            let _ = socket.send(Message::Text(msg.to_string())).await;
        }
        Err(e) => {
            let code = if e.to_string().contains("unique") { "CONFLICT" } else { "INTERNAL" };
            let err = serde_json::json!({"type":"error","code":code,
                "message":format!("Could not create channel: {e}")});
            let _ = socket.send(Message::Text(err.to_string())).await;
        }
    }
    Ok(())
}

```

**Step 5: Compile check**

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

Note: `send_as_human` is referenced but not yet defined — will be added in Task 8. Add a placeholder to allow compile:

```rust
// In webui_handlers.rs — placeholder, replaced in Task 8
pub async fn send_as_human(
    _pool: &PgPool, _router: &crate::router::Router,
    _ctx: &AgentContext, _channel_id: i64, _body: &str,
) -> Result<(), anyhow::Error> {
    Ok(())
}

```

**Step 6: Commit**

```bash
git add crates/synapse-broker/src/webui.rs \
        crates/synapse-broker/src/webui_handlers.rs \
        crates/synapse-broker/src/main.rs
git commit -m "feat(webui): WebSocket session auth + fleet channel init + command dispatch"

```

---

### Task 8: send_as_human — construct and publish Dialogue frame

**Files:**
- Modify: `crates/synapse-broker/src/webui_handlers.rs`

**Step 1: Read the frame.rs API to identify FrameHeader constructor**

```bash
grep -A10 "pub fn new\|pub fn to_bytes\|pub struct FrameHeader" \
  crates/synapse-proto/src/frame.rs | head -60

```

Also check how msg_loop.rs builds frames (it does this for Ack frames at least):

```bash
grep -A10 "FrameHeader" crates/synapse-broker/src/msg_loop.rs | head -40

```

**Step 2: Read router.publish signature**

```bash
grep -A5 "pub async fn publish\|pub fn publish" crates/synapse-broker/src/router.rs

```

**Step 3: Replace the placeholder send_as_human with the real implementation**

Adapt to the actual FrameHeader API found in Step 1. The key structure is:

```rust
use crate::router::Router;

pub async fn send_as_human(
    pool:       &PgPool,
    router:     &Router,
    ctx:        &AgentContext,
    channel_id: i64,
    body:       &str,
) -> Result<(), anyhow::Error> {
    // Payload: 0x01 (Dialogue) + 16-byte agent UUID + UTF-8 body
    let uuid_bytes = ctx.agent_uuid.as_bytes();
    let mut payload = Vec::with_capacity(1 + 16 + body.len());
    payload.push(0x01u8);
    payload.extend_from_slice(uuid_bytes);
    payload.extend_from_slice(body.as_bytes());

    // Generate message UUID
    let msg_uuid = uuid::Uuid::new_v4();

    // Build frame header — adapt field names/order to match the actual API
    // (read frame.rs in Step 1 before writing this)
    let hdr = FrameHeader::new(
        /* type, encoding, flags, priority, msg_uuid, channel_id, payload_len */
        // ... fill in from actual API ...
    );

    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&hdr.to_bytes());
    frame.extend_from_slice(&payload);

    // Persist to DB
    sqlx::query!(
        r#"INSERT INTO messages
           (message_uuid, channel_id, sender_id, content_type, body, compressed, priority)
           VALUES ($1, $2, $3, $4, $5, false, $6)"#,
        msg_uuid,
        channel_id,
        ctx.agent_id,
        1i32,
        body.as_bytes(),
        0i32
    )
    .execute(pool)
    .await?;

    // Publish to all channel subscribers
    router.publish(channel_id, frame).await;

    Ok(())
}

```

**Step 4: Compile check**

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

**Step 5: Commit**

```bash
git add crates/synapse-broker/src/webui_handlers.rs
git commit -m "feat(webui): send_as_human — Dialogue frame construction and router publish"

```

---

### Task 9: Full interactive frontend HTML

**Files:**
- Modify: `webui/index.html`

Replace the placeholder with the complete interactive WebUI. All DOM manipulation uses `textContent` and `createElement` — no `innerHTML` with server-controlled data to prevent XSS.

**Step 1: Write `webui/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Synapse</title>
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { background: #0d1117; color: #e6edf3; font-family: monospace;
       display: flex; height: 100vh; overflow: hidden; }

/* Sidebar */
#sidebar { width: 220px; min-width: 220px; background: #161b22;
           border-right: 1px solid #30363d; display: flex; flex-direction: column; }
#fleet-header { padding: 0.75rem 1rem; font-size: 0.75rem; color: #8b949e;
                letter-spacing: 0.1em; text-transform: uppercase;
                border-bottom: 1px solid #30363d; }
#channel-list { flex: 1; overflow-y: auto; padding: 0.5rem 0; }
.fleet-group-label { padding: 0.4rem 1rem; font-size: 0.7rem; color: #6e7681;
                     text-transform: uppercase; letter-spacing: 0.08em;
                     margin-top: 0.5rem; }
.channel-item { padding: 0.35rem 1rem; cursor: pointer; font-size: 0.85rem;
                color: #8b949e; display: flex; justify-content: space-between;
                align-items: center; }
.channel-item:hover { background: #21262d; color: #e6edf3; }
.channel-item.active { background: #1f6feb33; color: #58a6ff; }
.unread-badge { background: #f85149; color: #fff; border-radius: 999px;
                padding: 1px 6px; font-size: 0.7rem; min-width: 18px;
                text-align: center; }
#new-channel-btn { padding: 0.5rem 1rem; font-size: 0.8rem; color: #6e7681;
                   cursor: pointer; border-top: 1px solid #30363d;
                   background: none; text-align: left; width: 100%;
                   border-left: none; border-right: none; border-bottom: none; }
#new-channel-btn:hover { color: #58a6ff; background: #21262d; }

/* Main */
#main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
#channel-header { padding: 0.75rem 1rem; font-size: 0.9rem; color: #e6edf3;
                  border-bottom: 1px solid #30363d; background: #161b22; }
#messages { flex: 1; overflow-y: auto; padding: 1rem;
            display: flex; flex-direction: column; gap: 0.5rem; }
.msg { max-width: 70%; padding: 0.5rem 0.75rem; border-radius: 8px; }
.msg.them { align-self: flex-start; background: #21262d; }
.msg.me   { align-self: flex-end;   background: #1f6feb; }
.msg-meta { font-size: 0.7rem; color: #8b949e; margin-bottom: 0.2rem; }
.msg.me .msg-meta { text-align: right; }
.msg-body { font-size: 0.875rem; word-break: break-word; white-space: pre-wrap; }
#empty-state { color: #6e7681; font-size: 0.85rem; text-align: center;
               margin-top: 3rem; }

/* Input */
#input-bar { display: flex; gap: 0.5rem; padding: 0.75rem 1rem;
             border-top: 1px solid #30363d; background: #161b22; }
#msg-input { flex: 1; padding: 0.5rem 0.75rem; background: #0d1117;
             border: 1px solid #30363d; border-radius: 4px; color: #e6edf3;
             font-family: monospace; font-size: 0.875rem; resize: none;
             min-height: 38px; }
#msg-input:focus { outline: none; border-color: #58a6ff; }
#send-btn { padding: 0.5rem 1rem; background: #238636; border: none;
            border-radius: 4px; color: #fff; font-family: monospace;
            font-size: 0.875rem; cursor: pointer; white-space: nowrap; }
#send-btn:hover { background: #2ea043; }

/* Modal */
#modal-overlay { display: none; position: fixed; inset: 0; background: #00000088;
                 align-items: center; justify-content: center; }
#modal-overlay.open { display: flex; }
#modal { background: #161b22; border: 1px solid #30363d; border-radius: 8px;
         padding: 1.5rem; width: 320px; }
#modal h2 { font-size: 0.9rem; margin-bottom: 1rem; color: #58a6ff; }
#modal input { width: 100%; padding: 0.5rem; background: #0d1117;
               border: 1px solid #30363d; border-radius: 4px; color: #e6edf3;
               font-family: monospace; font-size: 0.875rem; margin-bottom: 0.75rem; }
#modal input:focus { outline: none; border-color: #58a6ff; }
#modal-actions { display: flex; gap: 0.5rem; justify-content: flex-end; }
#modal-actions button { padding: 0.4rem 0.9rem; border: none; border-radius: 4px;
                        font-family: monospace; font-size: 0.85rem; cursor: pointer; }
#modal-cancel { background: #21262d; color: #e6edf3; }
#modal-create { background: #238636; color: #fff; }
</style>
</head>
<body>

<div id="sidebar">
  <div id="fleet-header">SYNAPSE // <span id="fleet-label">...</span></div>
  <div id="channel-list"></div>
  <button id="new-channel-btn">+ New Channel</button>
</div>

<div id="main">
  <div id="channel-header"># <span id="active-channel-name">select a channel</span></div>
  <div id="messages"><p id="empty-state">Select a channel to begin.</p></div>
  <div id="input-bar">
    <textarea id="msg-input" rows="1" placeholder="Message..."></textarea>
    <button id="send-btn">Send</button>
  </div>
</div>

<div id="modal-overlay" role="dialog" aria-modal="true">
  <div id="modal">
    <h2>Create Channel</h2>
    <input id="modal-name" type="text" placeholder="#channel-name" autocomplete="off">
    <input id="modal-desc" type="text" placeholder="Description (optional)" autocomplete="off">
    <div id="modal-actions">
      <button id="modal-cancel">Cancel</button>
      <button id="modal-create">Create</button>
    </div>
  </div>
</div>

<script>
'use strict';

const _wsUrl = new URL('/ws', location.href);
_wsUrl.protocol = _wsUrl.protocol.replace('http', 'ws');
const WS_URL = _wsUrl.href;
const RECONNECT_DELAYS = [1000, 3000, 7000];
const HISTORY_LIMIT = 50;

let ws;
let myName = '';
let myFleet = '';
let allChannels = [];
let activeChannel = null;
let unread = {};
let reconnectAttempt = 0;

// --- WebSocket ---

function connect() {
  ws = new WebSocket(WS_URL);

  ws.addEventListener('open', () => { reconnectAttempt = 0; });

  ws.addEventListener('message', (e) => {
    let msg;
    try { msg = JSON.parse(e.data); } catch { return; }
    switch (msg.type) {
      case 'init':            handleInit(msg);           break;
      case 'history':         handleHistory(msg);        break;
      case 'message':         handleMessage(msg);        break;
      case 'channel_created': handleChannelCreated(msg); break;
      case 'error':           console.error('[synapse]', msg.code, msg.message); break;
    }
  });

  ws.addEventListener('close', (e) => {
    // Auth expiry arrives as 1006 (abnormal close) from failed HTTP 401 upgrade
    // Current: reconnect with backoff, then show connection-lost message
    if (e.code === 1008 || e.code === 4001) { location.href = '/login'; return; }
    if (reconnectAttempt < RECONNECT_DELAYS.length) {
      setTimeout(connect, RECONNECT_DELAYS[reconnectAttempt++]);
    } else {
      setChannelHeader('Connection lost. Reload to reconnect.');
    }
  });
}

function wsSend(cmd) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(cmd));
  }
}

// --- Event handlers ---

function handleInit(msg) {
  myName      = msg.agent.name;
  myFleet     = msg.agent.fleet;
  allChannels = msg.channels || [];
  document.getElementById('fleet-label').textContent = myFleet || 'No Fleet';
  renderChannelList();
  if (msg.default_channel) selectChannel(msg.default_channel);
}

function handleHistory(msg) {
  clearMessages();
  const messages = msg.messages || [];
  if (messages.length === 0) {
    showEmptyState('No messages yet.');
    return;
  }
  messages.forEach(m => appendMessage(m.sender, m.body, m.ts));
}

function handleMessage(msg) {
  if (msg.channel === activeChannel) {
    appendMessage(msg.sender || 'unknown', msg.body, msg.ts);
  } else {
    unread[msg.channel] = (unread[msg.channel] || 0) + 1;
    renderChannelList();
  }
}

function handleChannelCreated(msg) {
  allChannels.push({ id: msg.id, name: msg.name, fleet_name: msg.fleet });
  renderChannelList();
}

// --- Sidebar ---

function renderChannelList() {
  const box = document.getElementById('channel-list');
  // Clear safely
  while (box.firstChild) box.removeChild(box.firstChild);

  // Group channels by fleet
  const groups = {};
  allChannels.forEach(ch => {
    const fleet = ch.fleet_name || 'Unknown';
    if (!groups[fleet]) groups[fleet] = [];
    groups[fleet].push(ch);
  });

  // Own fleet first, shared fleets below with group label
  const ownFleet = myFleet;
  const sortedFleets = Object.keys(groups).sort((a, b) => {
    if (a === ownFleet) return -1;
    if (b === ownFleet) return  1;
    return a.localeCompare(b);
  });

  sortedFleets.forEach(fleetName => {
    if (fleetName !== ownFleet) {
      const label = document.createElement('div');
      label.className = 'fleet-group-label';
      label.textContent = fleetName;
      box.appendChild(label);
    }

    groups[fleetName].forEach(ch => {
      const el = document.createElement('div');
      el.className = 'channel-item' + (ch.name === activeChannel ? ' active' : '');

      const nameSpan = document.createElement('span');
      nameSpan.textContent = ch.name;
      el.appendChild(nameSpan);

      const count = unread[ch.name];
      if (count) {
        const badge = document.createElement('span');
        badge.className = 'unread-badge';
        badge.textContent = count;
        el.appendChild(badge);
      }

      el.addEventListener('click', () => selectChannel(ch.name));
      box.appendChild(el);
    });
  });
}

function selectChannel(name) {
  activeChannel = name;
  delete unread[name];
  setChannelHeader(name);
  renderChannelList();
  wsSend({ type: 'subscribe', channel: name });
}

function setChannelHeader(text) {
  document.getElementById('active-channel-name').textContent = text;
}

// --- Messages ---

function clearMessages() {
  const box = document.getElementById('messages');
  while (box.firstChild) box.removeChild(box.firstChild);
}

function showEmptyState(text) {
  const box = document.getElementById('messages');
  const p = document.createElement('p');
  p.id = 'empty-state';
  p.textContent = text;
  box.appendChild(p);
}

function appendMessage(sender, body, ts) {
  const box   = document.getElementById('messages');
  const empty = document.getElementById('empty-state');
  if (empty) box.removeChild(empty);

  const isMe = sender === myName;

  const el = document.createElement('div');
  el.className = 'msg ' + (isMe ? 'me' : 'them');

  const meta = document.createElement('div');
  meta.className = 'msg-meta';
  const time = ts ? new Date(ts).toLocaleTimeString() : '';
  meta.textContent = sender + (time ? '  ' + time : '');

  const bodyEl = document.createElement('div');
  bodyEl.className = 'msg-body';
  bodyEl.textContent = body;   // textContent — no XSS risk

  el.appendChild(meta);
  el.appendChild(bodyEl);
  box.appendChild(el);
  box.scrollTop = box.scrollHeight;
}

// --- Send ---

document.getElementById('send-btn').addEventListener('click', doSend);
document.getElementById('msg-input').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); doSend(); }
});

function doSend() {
  const input = document.getElementById('msg-input');
  const body  = input.value.trim();
  if (!body || !activeChannel) return;
  wsSend({ type: 'send', channel: activeChannel, body });
  input.value = '';
}

// --- New Channel Modal ---

document.getElementById('new-channel-btn').addEventListener('click', openModal);
document.getElementById('modal-cancel').addEventListener('click', closeModal);
document.getElementById('modal-overlay').addEventListener('click', (e) => {
  if (e.target === document.getElementById('modal-overlay')) closeModal();
});
document.getElementById('modal-create').addEventListener('click', () => {
  const name = document.getElementById('modal-name').value.trim();
  const desc = document.getElementById('modal-desc').value.trim();
  if (!name) return;
  wsSend({ type: 'create_channel', name, description: desc || null });
  closeModal();
});

function openModal() {
  document.getElementById('modal-overlay').classList.add('open');
  document.getElementById('modal-name').focus();
}

function closeModal() {
  document.getElementById('modal-overlay').classList.remove('open');
  document.getElementById('modal-name').value = '';
  document.getElementById('modal-desc').value = '';
}

// --- Boot ---
connect();
</script>
</body>
</html>

```

**Step 2: Compile check**

```bash
cargo build -p synapse-broker 2>&1 | grep "^error"

```

**Step 3: Commit**

```bash
git add webui/index.html
git commit -m "feat(webui): full interactive frontend — safe DOM API, fleet sidebar, send, channel create"

```

---

### Task 10: Bootstrap script — fleet and human operator setup

**Files:**
- Create: `scripts/bootstrap-fleet.sh`

**Step 1: Write `scripts/bootstrap-fleet.sh`**

```bash
#!/usr/bin/env bash
# Idempotent fleet bootstrap: create fleet, human operator agent, and default channel.
# Usage: ./scripts/bootstrap-fleet.sh <fleet-name> <agent-name> <secret> [default-channel]
# Example: ./scripts/bootstrap-fleet.sh lex commander mysecret '#general'
set -euo pipefail

FLEET_NAME="${1:?Usage: $0 <fleet-name> <agent-name> <secret> [default-channel]}"
AGENT_NAME="${2:?}"
AGENT_SECRET="${3:?}"
DEFAULT_CHANNEL="${4:-#general}"

PSQL="docker exec -i stratavore-postgres psql -U postgres -d synapse -v ON_ERROR_STOP=1"

echo "[bootstrap-fleet] Fleet='${FLEET_NAME}' Agent='${AGENT_NAME}' Channel='${DEFAULT_CHANNEL}'"

$PSQL \
  -v "fleet_name=${FLEET_NAME}" \
  -v "agent_name=${AGENT_NAME}" \
  -v "agent_secret=${AGENT_SECRET}" \
  -v "channel_name=${DEFAULT_CHANNEL}" \
  <<'SQL'
DO $$
DECLARE
  v_agent_id   BIGINT;
  v_fleet_id   BIGINT;
  v_channel_id BIGINT;
BEGIN
  -- Upsert human agent (:'var' uses psql quoting — safe against injection)
  INSERT INTO agents (name, secret_hash, is_human)
  VALUES (:'agent_name', :'agent_secret', true)
  ON CONFLICT (name)
  DO UPDATE SET secret_hash = EXCLUDED.secret_hash, is_human = true
  RETURNING id INTO v_agent_id;

  -- Idempotent fleet: insert or find existing (do not overwrite owner on conflict)
  INSERT INTO fleets (name, owner_id)
  VALUES (:'fleet_name', v_agent_id)
  ON CONFLICT (name) DO NOTHING
  RETURNING id INTO v_fleet_id;
  IF v_fleet_id IS NULL THEN
    SELECT id INTO v_fleet_id FROM fleets WHERE name = :'fleet_name';
  END IF;

  -- Assign agent to fleet
  UPDATE agents SET fleet_id = v_fleet_id WHERE id = v_agent_id;

  -- Idempotent channel: select existing fleet channel or insert new one
  SELECT id INTO v_channel_id FROM channels
    WHERE name = :'channel_name' AND fleet_id = v_fleet_id;
  IF v_channel_id IS NULL THEN
    INSERT INTO channels (name, fleet_id, created_by)
    VALUES (:'channel_name', v_fleet_id, v_agent_id)
    RETURNING id INTO v_channel_id;
  END IF;

  -- Set default channel on agent
  UPDATE agents SET default_channel_id = v_channel_id WHERE id = v_agent_id;

  RAISE NOTICE 'Done: fleet_id=% agent_id=% channel_id=%',
    v_fleet_id, v_agent_id, v_channel_id;
END;
$$;
SQL

echo "[bootstrap-fleet] Complete."

```

**Step 2: Make executable**

```bash
chmod +x scripts/bootstrap-fleet.sh

```

**Step 3: Run for the Lex fleet**

```bash
COMMANDER_SECRET=$(docker exec stratavore-postgres psql -U postgres -d synapse \
  -tAc "SELECT secret_hash FROM agents WHERE name='commander'")

./scripts/bootstrap-fleet.sh lex commander "${COMMANDER_SECRET}" '#general'

```

**Step 4: Verify**

```bash
docker exec stratavore-postgres psql -U postgres -d synapse -c \
  "SELECT a.name, a.is_human, f.name AS fleet, c.name AS default_channel
   FROM agents a
   LEFT JOIN fleets f ON f.id = a.fleet_id
   LEFT JOIN channels c ON c.id = a.default_channel_id
   WHERE a.name = 'commander'"

```

Expected: 1 row with fleet=lex, default_channel=#general, is_human=t

**Step 5: Commit**

```bash
git add scripts/bootstrap-fleet.sh
git commit -m "feat(scripts): bootstrap-fleet.sh — idempotent fleet/human-agent/channel setup"

```

---

### Task 11: Full build, smoke test, and PR

**Step 1: Final compile**

```bash
cargo build --release -p synapse-broker 2>&1 | grep "^error"

```

Expected: 0 errors.

**Step 2: Run tests**

```bash
cargo test --workspace 2>&1 | tail -20

```

Expected: all tests pass.

**Step 3: Deploy local binary to running container**

```bash
cargo build --release -p synapse-broker

docker cp target/release/synapse-broker stratavore-synapse:/usr/local/bin/synapse-broker
docker restart stratavore-synapse
docker logs stratavore-synapse --tail=20

```

Expected: `Applied 1 migration(s)`, broker starts.

**Step 4: Run bootstrap script**

```bash
COMMANDER_SECRET=$(docker exec stratavore-postgres psql -U postgres -d synapse \
  -tAc "SELECT secret_hash FROM agents WHERE name='commander'")
./scripts/bootstrap-fleet.sh lex commander "${COMMANDER_SECRET}" '#general'

```

**Step 5: Smoke test**

1. Open `http://<host>:7778/` in browser
2. Verify redirect to `/login`
3. Login with `commander` + secret
4. Verify redirect to `/` with fleet sidebar showing `#general`
5. Send a message — verify it appears in message area
6. In a terminal: `synapse listen --channel "#general"` — verify message arrives

**Step 6: Create PR**

```bash
git push origin feat/fleet-webui
gh pr create \
  --repo Meridian-Lex/Synapse \
  --head Meridian-Lex:feat/fleet-webui \
  --assignee LunarLaurus \
  --title "feat: fleet WebUI — interactive multi-tenant chat platform" \
  --body "$(cat <<'EOF'
## Summary

- Migration 002: fleets, fleet_shares tables; fleet_id/is_human/default_channel_id/agent_uuid columns
- HTTP session auth: GET/POST /login with cookie, 7-day expiry, redirect guard on /
- WebSocket JSON protocol: subscribe, send, create_channel commands; init/history/message/channel_created events
- Interactive frontend: fleet sidebar, channel switching, message history, send, channel create modal — all DOM built with createElement/textContent (no innerHTML with user data)
- send_as_human: constructs binary Dialogue frame and publishes via router — binary agents receive transparently
- Cross-fleet channel visibility via fleet_shares bilateral SQL query
- bootstrap-fleet.sh: idempotent fleet/human-agent/default-channel setup script

## Test plan

- [ ] Migration 002 applies cleanly; rollback clean
- [ ] POST /login valid credentials: cookie set, redirect to /
- [ ] POST /login wrong secret: 401
- [ ] GET / without cookie: redirect to /login
- [ ] WS /ws without cookie: 401
- [ ] Init message contains correct agent/fleet/channels/default_channel
- [ ] Subscribe command: history sent, subscription active
- [ ] Send from WebUI: synapse-cli listener receives Dialogue frame
- [ ] Channel create: appears in sidebar, exists in DB
- [ ] Cross-fleet channel visible after fleet_shares insert
- [ ] Expired session: WS upgrade rejected HTTP 401; client enters reconnect loop then shows "Connection lost. Reload to reconnect."

EOF
)"

```

---

### Task 12: Ralph Loop — PR review cycle (CodeRabbit + Sourcery)

**Reference**: `lex-internal/knowledge/review-resolution-workflow.md`

This task runs after Task 11 (PR created). It is cyclic — repeat until `reviewDecision=APPROVED` and `mergeStateStatus=CLEAN`.

---

**Step 1: Trigger CodeRabbit review (Cycle 1)**

Post a review request comment on the PR:

```bash
gh api --method POST \
  repos/Meridian-Lex/Synapse/issues/<PR_NUMBER>/comments \
  -f body='@coderabbitai review'

```

**Step 2: Cryopod — wait for review completion**

```bash
sleep 1800   # 30 minutes — CodeRabbit + Sourcery both need time to complete

```

Check for activity if needed:

```bash
gh api repos/Meridian-Lex/Synapse/pulls/<PR_NUMBER>/reviews \
  --jq '.[] | {user: .user.login, state: .state, submitted_at: .submitted_at}' \
  | jq -s 'sort_by(.submitted_at) | reverse | .[0:5]'

```

**Step 3: Extract all review threads and AI agent prompts**

```bash
# Get all review thread IDs and resolution status
gh api graphql -f query='
{
  repository(owner: "Meridian-Lex", name: "Synapse") {
    pullRequest(number: <PR_NUMBER>) {
      reviewThreads(first: 50) {
        nodes {
          id
          isResolved
          comments(first: 1) {
            nodes { body path line }
          }
        }
      }
    }
  }
}'

```

```bash
# Get latest coderabbitai reviews with full body (AI agent prompts live here)
gh api repos/Meridian-Lex/Synapse/pulls/<PR_NUMBER>/reviews \
  --jq '.[] | select(.user.login == "coderabbitai[bot]") | {id: .id, state: .state, body: .body, submitted_at: .submitted_at}' \
  | jq -s 'sort_by(.submitted_at) | reverse | .[0:2]'

```

Look for:
- `<details><summary>🤖 Prompt for all review comments with AI agents</summary>` blocks
- Individual `🤖 Prompt for AI Agents` sections on inline comments

**Step 4: Triage all open threads**

For each unresolved thread, decide:
- **Fix**: Code change required — apply the fix
- **Acknowledge**: Not applicable to current code, or design decision — reply explaining why, still resolve the thread

**ALWAYS verify against current code before applying a fix** — reviews may be stale if commits landed after the review was posted.

**Step 5: Apply fixes and commit**

For each fixable finding:

```bash
# After making the code change
git add <changed files>
git commit -m "fix: <description of finding resolved>"
git push origin feat/fleet-webui

```

Group related fixes into a single commit where sensible. One commit per logical fix — not one per thread.

**Step 6: Reply to all open review threads**

Get inline comment IDs:

```bash
gh api repos/Meridian-Lex/Synapse/pulls/<PR_NUMBER>/comments \
  --jq '.[] | select(.user.login == "coderabbitai[bot]") | {id: .id, path: .path, line: .line}'

```

Reply to each thread with resolution confirmation:

```bash
gh api --method POST \
  repos/Meridian-Lex/Synapse/pulls/<PR_NUMBER>/comments/<COMMENT_ID>/replies \
  -f body='**RESOLVED** - [concise description of fix applied, or reason not applicable]'

```

Reply format:
- Fixed: `**RESOLVED** - <file>:<line> <what was changed>.`
- Not applicable: `**ACKNOWLEDGED** - <why this does not apply to current code>.`
- Design decision: `**ACKNOWLEDGED** - <rationale for intentional design choice>.`

**Step 7: Resolve all threads via GraphQL**

For each thread ID retrieved in Step 3:

```bash
gh api graphql -f query='
mutation {
  resolveReviewThread(input: {threadId: "<THREAD_ID>"}) {
    thread { isResolved }
  }
}'

```

Verify all threads resolved:

```bash
gh api graphql -f query='
{
  repository(owner: "Meridian-Lex", name: "Synapse") {
    pullRequest(number: <PR_NUMBER>) {
      reviewThreads(first: 50) {
        nodes { id isResolved }
      }
    }
  }
}' | jq '.data.repository.pullRequest.reviewThreads.nodes | map(select(.isResolved == false)) | length'

```

Expected: `0`

**Step 8: Check PR approval state**

```bash
gh pr view <PR_NUMBER> --repo Meridian-Lex/Synapse \
  --json reviewDecision,mergeStateStatus,reviews \
  | jq '{reviewDecision, mergeStateStatus}'

```

- `reviewDecision=APPROVED` + `mergeStateStatus=CLEAN` → proceed to Step 10
- `reviewDecision=REVIEW_REQUIRED` or findings remain → trigger Cycle 2 (Step 9)

**Step 9: Trigger Cycle 2+ (if needed)**

```bash
gh api --method POST \
  repos/Meridian-Lex/Synapse/issues/<PR_NUMBER>/comments \
  -f body='@coderabbitai review'

sleep 1800  # reduced to 15 min for subsequent cycles if PR is quiet

```

Repeat Steps 3–8 until clean.

**Step 10: Enable auto-merge and confirm**

```bash
gh pr merge <PR_NUMBER> \
  --repo Meridian-Lex/Synapse \
  --auto --merge

# Confirm merge state
gh pr view <PR_NUMBER> --repo Meridian-Lex/Synapse \
  --json state,mergedAt,mergeStateStatus | jq .

```

If conditions already met (APPROVED + CLEAN), auto-merge triggers immediately. Confirm `state=MERGED`.

**Step 11: Post-merge — update task queue**

Update `lex-internal/state/TASK-QUEUE.md`:
- Task 61: change `IN PROGRESS` → `COMPLETE`, add `Merged: <timestamp>`, `PR: <url>`

```bash
cd /home/meridian/meridian-home/lex-internal
git add state/TASK-QUEUE.md
git commit -m "chore: Task 61 complete — Synapse Fleet WebUI merged"
git push origin master

```

---

## Implementation Notes

**FrameHeader API**: Before implementing `send_as_human` (Task 8), read `crates/synapse-proto/src/frame.rs` to confirm the exact constructor signature and `to_bytes` method. Adapt the call accordingly.

**Router.publish signature**: Check `crates/synapse-broker/src/router.rs` for the exact `publish` method signature before calling it.

**messages.body is BYTEA**: Store `body.as_bytes()` on insert, decode as UTF-8 on read. The history query handles this via `String::from_utf8_lossy`.

**channels.description**: The existing schema may or may not have a `description` column. Check migration 001 before using it in `create_channel`.

**sqlx query macros**: These are checked at compile time against the DB. If the DB is not running during `cargo build`, use `cargo build --offline` or set `SQLX_OFFLINE=true` with a pre-generated `.sqlx/` directory. Generate with `cargo sqlx prepare --workspace`.
