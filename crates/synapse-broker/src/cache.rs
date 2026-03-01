use anyhow::Result;
use redis::{aio::MultiplexedConnection, AsyncCommands};

const PRESENCE_TTL: u64 = 30;

pub async fn connect(url: &str) -> Result<MultiplexedConnection> {
    let conn = redis::Client::open(url)?.get_multiplexed_async_connection().await?;
    tracing::info!("Redis connected");
    Ok(conn)
}

pub async fn set_presence(c: &mut MultiplexedConnection, id: i64) -> Result<()> {
    c.set_ex::<_, _, ()>(format!("synapse:presence:{}", id), "online", PRESENCE_TTL).await?;
    Ok(())
}

pub async fn remove_presence(c: &mut MultiplexedConnection, id: i64) -> Result<()> {
    c.del::<_, ()>(format!("synapse:presence:{}", id)).await?;
    Ok(())
}

pub async fn cache_session(c: &mut MultiplexedConnection, token: &str, id: i64, ttl: u64) -> Result<()> {
    c.set_ex::<_, _, ()>(format!("synapse:session:{}", token), id.to_string(), ttl).await?;
    Ok(())
}

pub async fn publish_message(c: &mut MultiplexedConnection, channel_id: i64, frame: &[u8]) -> Result<()> {
    c.publish::<_, _, ()>(format!("synapse:channel:{}", channel_id), frame).await?;
    Ok(())
}
