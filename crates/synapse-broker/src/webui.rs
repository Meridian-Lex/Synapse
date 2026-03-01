use crate::router::Router;
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    routing::get,
    Router as AxumRouter,
};
use std::sync::Arc;
use synapse_proto::frame::{Encoding, FrameHeader, HEADER_LEN};
use synapse_proto::compression::decompress;
use tracing::warn;

pub fn build_router(router: Arc<Router>) -> AxumRouter {
    AxumRouter::new()
        .route("/ws", get(ws_handler))
        .route("/",   get(serve_index))
        .with_state(router)
}

async fn serve_index() -> impl IntoResponse {
    axum::response::Html(include_str!("../../../webui/index.html"))
}

async fn ws_handler(ws: WebSocketUpgrade, State(router): State<Arc<Router>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, router))
}

async fn handle_ws(mut socket: WebSocket, router: Arc<Router>) {
    // Subscribe to #general (channel id 1, seeded in migration)
    let mut rx = router.subscribe(1).await;
    loop {
        tokio::select! {
            Ok(frame) = rx.recv() => {
                if frame.len() < HEADER_LEN {
                    continue;
                }

                // Parse the frame header using the proper API
                let header_bytes: &[u8; HEADER_LEN] = match frame[..HEADER_LEN].try_into() {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let hdr = match FrameHeader::from_bytes(header_bytes) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let raw_payload = &frame[HEADER_LEN..];

                // Decompress if needed
                let payload: Vec<u8> = if hdr.encoding == Encoding::Zstd {
                    match decompress(raw_payload) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("webui: failed to decompress frame {}: {}", hdr.message_id, e);
                            continue;
                        }
                    }
                } else {
                    raw_payload.to_vec()
                };

                // content_type 0x01 = DIALOGUE; body starts at payload[17..]
                // payload[0] = content_type, payload[1..17] = 16-byte sender UUID
                if payload.len() > 17 && payload[0] == 0x01 {
                    if let Ok(text) = std::str::from_utf8(&payload[17..]) {
                        let json = serde_json::json!({ "type": "message", "body": text }).to_string();
                        if socket.send(Message::Text(json.into())).await.is_err() { break; }
                    }
                }
            }
            Some(Ok(_msg)) = socket.recv() => {
                // Human observer input reserved for future task
            }
            else => break,
        }
    }
}
