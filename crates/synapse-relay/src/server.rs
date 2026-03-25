use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing;

use crate::relay::RelayRegistry;
use crate::types::*;

pub type AppState = Arc<RelayRegistry>;

// Query parameter structs
#[derive(Debug, Deserialize)]
struct PollQuery {
    channel: Option<String>,
    since: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WaitQuery {
    channel: Option<String>,
    since: Option<u64>,
    min: Option<usize>,
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UsersQuery {
    channel: Option<String>,
}

// Response helpers
fn ok_resp() -> (StatusCode, Json<OkResp>) {
    (StatusCode::OK, Json(OkResp { ok: true }))
}

// Response enum for mixed success/error handlers
#[derive(Serialize)]
#[serde(untagged)]
enum MixedResp {
    Ok(OkResp),
    Error(ErrorResp),
}

// Calculate next_seq from messages
fn next_seq_from(msgs: &[BufferedMessage], since: u64) -> u64 {
    msgs.last().map(|m| m.seq + 1).unwrap_or(since + 1)
}

// Handlers
async fn handle_subscribe(
    State(registry): State<AppState>,
    Json(req): Json<SubscribeReq>,
) -> (StatusCode, Json<MixedResp>) {
    match registry.subscribe(&req.channel).await {
        Ok(()) => (StatusCode::OK, Json(MixedResp::Ok(OkResp { ok: true }))),
        Err(e) => {
            tracing::error!("subscribe failed for channel {}: {}", req.channel, e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MixedResp::Error(ErrorResp {
                    error: e.to_string(),
                    channel: Some(req.channel),
                })),
            )
        }
    }
}

async fn handle_leave(
    State(registry): State<AppState>,
    Json(req): Json<LeaveReq>,
) -> (StatusCode, Json<OkResp>) {
    registry.leave(&req.channel);
    ok_resp()
}

async fn handle_send(
    State(registry): State<AppState>,
    Json(req): Json<SendReq>,
) -> (StatusCode, Json<MixedResp>) {
    match registry.send_dialogue(&req.channel, &req.text).await {
        Ok(()) => (StatusCode::OK, Json(MixedResp::Ok(OkResp { ok: true }))),
        Err(e) => {
            tracing::error!("send failed for channel {}: {}", req.channel, e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MixedResp::Error(ErrorResp {
                    error: e.to_string(),
                    channel: Some(req.channel),
                })),
            )
        }
    }
}

async fn handle_send_work(
    State(registry): State<AppState>,
    Json(req): Json<SendWorkReq>,
) -> (StatusCode, Json<MixedResp>) {
    match registry.send_work(&req.channel, req.payload).await {
        Ok(()) => (StatusCode::OK, Json(MixedResp::Ok(OkResp { ok: true }))),
        Err(e) => {
            tracing::error!("send_work failed for channel {}: {}", req.channel, e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MixedResp::Error(ErrorResp {
                    error: e.to_string(),
                    channel: Some(req.channel),
                })),
            )
        }
    }
}

async fn handle_poll(
    State(registry): State<AppState>,
    Query(q): Query<PollQuery>,
) -> impl IntoResponse {
    let channel = match q.channel {
        Some(ch) => ch,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(MixedResp::Error(ErrorResp {
                    error: "channel is required".to_string(),
                    channel: None,
                })),
            )
                .into_response();
        }
    };

    let since = q.since.unwrap_or(0);
    let messages = registry.poll(&channel, since);
    let next_seq = next_seq_from(&messages, since);

    (
        StatusCode::OK,
        Json(PollResp {
            messages,
            next_seq,
        }),
    )
        .into_response()
}

async fn handle_wait(
    State(registry): State<AppState>,
    Query(q): Query<WaitQuery>,
) -> impl IntoResponse {
    let channel = match q.channel {
        Some(ch) => ch,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(MixedResp::Error(ErrorResp {
                    error: "channel is required".to_string(),
                    channel: None,
                })),
            )
                .into_response();
        }
    };

    let since = q.since.unwrap_or(0);
    let min = q.min.unwrap_or(1);
    let timeout = q.timeout.unwrap_or(30000);

    let (messages, timed_out) = match registry.wait(&channel, since, min, timeout).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("wait failed: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(MixedResp::Error(ErrorResp {
                    error: format!("wait failed: {}", e),
                    channel: Some(channel),
                })),
            )
                .into_response();
        }
    };
    let next_seq = next_seq_from(&messages, since);

    (
        StatusCode::OK,
        Json(WaitResp {
            messages,
            next_seq,
            timed_out,
        }),
    )
        .into_response()
}

async fn handle_channels(State(registry): State<AppState>) -> impl IntoResponse {
    match registry.list_channels().await {
        Ok(channels) => (
            StatusCode::OK,
            Json(ChannelsResp { channels }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("list_channels failed: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MixedResp::Error(ErrorResp {
                    error: e.to_string(),
                    channel: None,
                })),
            )
                .into_response()
        }
    }
}

async fn handle_users(
    State(registry): State<AppState>,
    Query(q): Query<UsersQuery>,
) -> impl IntoResponse {
    let channel = match q.channel {
        Some(ch) => ch,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(MixedResp::Error(ErrorResp {
                    error: "channel is required".to_string(),
                    channel: None,
                })),
            )
                .into_response();
        }
    };

    match registry.list_users(&channel).await {
        Ok(users) => (
            StatusCode::OK,
            Json(UsersResp { users }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("list_users failed for channel {}: {}", channel, e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MixedResp::Error(ErrorResp {
                    error: e.to_string(),
                    channel: Some(channel),
                })),
            )
                .into_response()
        }
    }
}

async fn handle_status(State(registry): State<AppState>) -> (StatusCode, Json<StatusResp>) {
    let channels = registry.status();
    (StatusCode::OK, Json(StatusResp { channels }))
}

/// Initialize and run the axum HTTP server.
pub async fn run(bind: String, port: u16) -> anyhow::Result<()> {
    let config = crate::config::Config::load()?;
    config.validate()?;
    let registry = Arc::new(RelayRegistry::new(config));

    let app = Router::new()
        .route("/subscribe", post(handle_subscribe))
        .route("/leave", post(handle_leave))
        .route("/send", post(handle_send))
        .route("/send_work", post(handle_send_work))
        .route("/poll", get(handle_poll))
        .route("/wait", get(handle_wait))
        .route("/channels", get(handle_channels))
        .route("/users", get(handle_users))
        .route("/status", get(handle_status))
        .with_state(registry);

    let addr = format!("{}:{}", bind, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("synapse-relay listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::Response;
    use crate::config::Config;

    fn test_config() -> Config {
        Config {
            agent_name: "test-agent".into(),
            secret: "test-secret".into(),
            ..Config::default()
        }
    }

    fn test_registry() -> AppState {
        Arc::new(RelayRegistry::new(test_config()))
    }

    // Helper to extract response status and body
    async fn response_status_and_body(
        resp: impl IntoResponse,
    ) -> (StatusCode, String) {
        let resp: Response = resp.into_response();
        let status = resp.status();
        let body = to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        let body_str = String::from_utf8_lossy(&body).to_string();
        (status, body_str)
    }

    #[tokio::test]
    async fn test_status_returns_empty_channels() {
        let registry = test_registry();
        let resp = handle_status(State(registry)).await;
        let (status, body) = response_status_and_body(resp).await;

        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["channels"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_poll_missing_channel_returns_400() {
        let registry = test_registry();
        let q = PollQuery {
            channel: None,
            since: None,
        };
        let resp = handle_poll(State(registry), Query(q)).await;
        let (status, _body) = response_status_and_body(resp).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_poll_unsubscribed_channel_returns_empty() {
        let registry = test_registry();
        let q = PollQuery {
            channel: Some("test-channel".to_string()),
            since: None,
        };
        let resp = handle_poll(State(registry), Query(q)).await;
        let (status, body) = response_status_and_body(resp).await;

        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["messages"].as_array().unwrap().len(), 0);
        assert_eq!(json["next_seq"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_users_missing_channel_returns_400() {
        let registry = test_registry();
        let q = UsersQuery { channel: None };
        let resp = handle_users(State(registry), Query(q)).await;
        let (status, _body) = response_status_and_body(resp).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_leave_always_succeeds() {
        let registry = test_registry();
        let req = LeaveReq {
            channel: "any-channel".to_string(),
        };
        let resp = handle_leave(State(registry), Json(req)).await;
        let (status, _body) = response_status_and_body(resp).await;

        assert_eq!(status, StatusCode::OK);
    }
}
