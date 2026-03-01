use crate::router::Router;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Form, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
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

// --- Session helpers ---

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
    row.agent_id
}

// --- GET / ---

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

// --- GET /login ---

async fn serve_login() -> impl IntoResponse {
    Html(include_str!("../../../webui/login.html").replace("{{ERROR}}", ""))
}

fn login_error_response(msg: &str) -> Response {
    let safe_msg = msg.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let body = include_str!("../../../webui/login.html")
        .replace("{{ERROR}}", &format!(r#"<p class="error">{safe_msg}</p>"#));
    (StatusCode::UNAUTHORIZED, Html(body)).into_response()
}

// --- POST /login ---

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

    let agent = match agent {
        Err(e) => {
            warn!("webui: login DB error for agent '{}': {}", form.name, e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => return login_error_response("Invalid agent name or secret."),
        Ok(Some(a)) => a,
    };

    // Constant-time byte comparison to prevent timing-oracle attacks.
    // secret_hash stores fixed-width comparison tokens (internal API keys),
    // not bcrypt/argon2 hashes — no KDF is needed for this trust boundary.
    if !constant_time_eq(agent.secret_hash.as_bytes(), form.secret.as_bytes()) {
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

    if let Err(e) = insert {
        warn!("webui: session insert failed for agent '{}': {}", form.name, e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
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

// --- WebSocket ---

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
    let mut channels = match ctx.fleet_id {
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
        "channels":        channels,
        "default_channel": default_channel,
    });

    if socket.send(Message::Text(init.to_string())).await.is_err() {
        return;
    }

    let mut rx: Option<tokio::sync::broadcast::Receiver<Vec<u8>>> = None;
    let mut fleet_rx: Option<tokio::sync::broadcast::Receiver<()>> = match ctx.fleet_id {
        Some(fid) => Some(state.broker_router.subscribe_fleet(fid).await),
        None => None,
    };
    let mut active_channel_id: Option<i64> = None;

    loop {
        tokio::select! {
            result = channel_recv(&mut rx) => {
                match result {
                    Ok(frame) => {
                        if let Some(json) = crate::webui_handlers::frame_to_json(
                            &frame, &state.pool, active_channel_id, &channels,
                        ).await {
                            if socket.send(Message::Text(json)).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("webui: broadcast lagged, dropped {} messages", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        warn!("webui: broadcast channel closed");
                        break;
                    }
                }
            }
            event = fleet_recv(&mut fleet_rx) => {
                match event {
                    Ok(()) => {
                        if let Some(fid) = ctx.fleet_id {
                            channels = crate::webui_handlers::fetch_channel_list(&state.pool, fid).await;
                            let upd = serde_json::json!({
                                "type": "channel_list_updated",
                                "channels": channels,
                            });
                            if socket.send(Message::Text(upd.to_string())).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("webui: fleet broadcast lagged, dropped {} events", n);
                        // Refresh anyway — at least one event was missed
                        if let Some(fid) = ctx.fleet_id {
                            channels = crate::webui_handlers::fetch_channel_list(&state.pool, fid).await;
                            let upd = serde_json::json!({
                                "type": "channel_list_updated",
                                "channels": channels,
                            });
                            if socket.send(Message::Text(upd.to_string())).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Fleet broadcast channel gone; disable future polling to avoid spin
                        fleet_rx = None;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(m)) => {
                        if dispatch_command(m, &mut socket, &state, &ctx, &mut channels,
                                           &mut rx, &mut active_channel_id).await.is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
    }
}

/// Constant-time byte equality. Prevents timing-oracle attacks when comparing
/// secrets: all bytes are always compared regardless of where the first
/// difference occurs. Returns false immediately if lengths differ (length is
/// not secret for our fixed-width internal tokens).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Await a message from an optional broadcast receiver.
/// Yields std::future::pending when the receiver is None, allowing select! to
/// keep the branch live without ever firing it.
async fn channel_recv(
    rx: &mut Option<tokio::sync::broadcast::Receiver<Vec<u8>>>,
) -> Result<Vec<u8>, tokio::sync::broadcast::error::RecvError> {
    match rx {
        Some(r) => r.recv().await,
        None    => std::future::pending::<Result<Vec<u8>, tokio::sync::broadcast::error::RecvError>>().await,
    }
}

/// Like channel_recv but for fleet-level unit signals.
/// Returns Result so the caller can distinguish Closed (disable polling)
/// from Lagged (missed events but still alive) rather than silently
/// converting both to None and risking a tight select! spin.
async fn fleet_recv(
    rx: &mut Option<tokio::sync::broadcast::Receiver<()>>,
) -> Result<(), tokio::sync::broadcast::error::RecvError> {
    match rx {
        Some(r) => r.recv().await,
        None    => std::future::pending::<Result<(), tokio::sync::broadcast::error::RecvError>>().await,
    }
}

// --- Command dispatch ---

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
    channels:          &mut Vec<crate::webui_handlers::ChannelInfo>,
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
            handle_create_channel(name, description, socket, state, ctx, channels).await
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
    if body.trim().is_empty() {
        return Ok(());
    }
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
    channels:    &mut Vec<crate::webui_handlers::ChannelInfo>,
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
            // Signal all fleet sessions that the channel list has changed
            state.broker_router.notify_fleet(fleet_id).await;
            // Update session channel list so subscribe/send work immediately
            channels.push(ch);
        }
        Err(e) => {
            let is_conflict = matches!(&e,
                anyhow::Error { .. } if e.downcast_ref::<sqlx::Error>()
                    .and_then(|se| se.as_database_error())
                    .and_then(|db| db.code())
                    .map(|code| code == "23505")
                    .unwrap_or(false)
            );
            let code    = if is_conflict { "CONFLICT" } else { "INTERNAL" };
            let message = if is_conflict { "Channel name already exists." }
                          else           { "Could not create channel. Please try again." };
            warn!("webui: create_channel error: {}", e);
            let err = serde_json::json!({"type":"error","code":code,"message":message});
            let _ = socket.send(Message::Text(err.to_string())).await;
        }
    }
    Ok(())
}
