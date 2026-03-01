use crate::router::Router;
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
    routing::get,
    Router as AxumRouter,
};
use std::sync::Arc;

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
                // Forward only DIALOGUE frames (content_type = 0x01 at byte 16)
                if frame.len() > 17 && frame[16] == 0x01 {
                    if let Ok(text) = std::str::from_utf8(&frame[33..]) {
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
