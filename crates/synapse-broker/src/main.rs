mod cache;
mod config;
mod connection;
mod db;
mod msg_loop;
mod router;
mod tls;
mod webui;

use std::sync::Arc;
use tokio::{net::TcpListener, sync::Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let path = std::env::var("SYNAPSE_CONFIG").unwrap_or_else(|_| "configs/synapse.yaml".into());
    let cfg = config::load(&path)?;

    let pool     = db::connect(&cfg.postgres.url).await?;
    let redis    = Arc::new(Mutex::new(cache::connect(&cfg.redis.url).await?));
    let router   = router::Router::default();
    let acceptor = tls::build_acceptor(&cfg.broker.tls_cert, &cfg.broker.tls_key)?;

    if cfg.webui.enabled {
        let wr = Arc::new(router.clone());
        let addr: std::net::SocketAddr = cfg.webui.listen.parse()?;
        tracing::info!("WebUI on http://{}", addr);
        tokio::spawn(async move {
            axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(),
                        webui::build_router(wr))
                .await.unwrap();
        });
    }

    let listener = TcpListener::bind(&cfg.broker.listen).await?;
    tracing::info!("Synapse broker on {}", cfg.broker.listen);

    loop {
        let (tcp, peer) = listener.accept().await?;
        tracing::info!("Connection from {}", peer);
        let (acceptor, pool, redis, router, ttl) =
            (acceptor.clone(), pool.clone(), redis.clone(), router.clone(), cfg.broker.session_ttl_seconds);

        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                Ok(mut tls) => match connection::handshake(&mut tls, &pool, &redis, ttl).await {
                    Ok(agent) => {
                        if let Err(e) = msg_loop::run(&mut tls, &agent, &pool, &redis, &router).await {
                            tracing::warn!("Session error for {}: {}", agent.agent_name, e);
                        }
                    }
                    Err(e) => tracing::warn!("Auth failed from {}: {}", peer, e),
                },
                Err(e) => tracing::warn!("TLS failed from {}: {}", peer, e),
            }
        });
    }
}
