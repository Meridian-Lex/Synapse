use crate::{cache, db};
use anyhow::Result;
use rand::RngCore;
use sqlx::PgPool;
use std::sync::Arc;
use synapse_proto::{
    auth::{verify_hmac, HelloPayload},
    codec::{read_frame, write_frame},
    frame::{FrameHeader, MsgType},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use redis::aio::MultiplexedConnection;

#[allow(dead_code)]  // session_token is stored for future session-revocation and audit use
pub struct AuthenticatedAgent {
    pub agent_id:      i64,
    pub agent_name:    String,
    pub session_token: String,
}

pub async fn handshake<S>(
    stream: &mut S,
    pool: &PgPool,
    redis: &Arc<Mutex<MultiplexedConnection>>,
    session_ttl: u64,
) -> Result<AuthenticatedAgent>
where S: AsyncRead + AsyncWrite + Unpin,
{
    // Read HELLO — 5-second deadline guards against slow/malicious clients
    let (hdr, payload) = timeout(Duration::from_secs(5), read_frame(stream)).await??;
    anyhow::ensure!(hdr.msg_type == MsgType::Hello, "expected HELLO");
    let hello = HelloPayload::decode(&payload)?;
    tracing::info!("HELLO from {}", hello.agent_name);

    // Look up agent secret
    let secret = db::get_agent_secret(pool, &hello.agent_name).await?
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {}", hello.agent_name))?;

    // Send CHALLENGE
    let mut nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    write_frame(stream, &FrameHeader::new(MsgType::Challenge, rand::random(), 32), &nonce).await?;

    // Read HELLO_RESP — 5-second deadline
    let (resp_hdr, resp) = timeout(Duration::from_secs(5), read_frame(stream)).await??;
    anyhow::ensure!(resp_hdr.msg_type == MsgType::HelloResp, "expected HELLO_RESP");

    if !verify_hmac(secret.as_bytes(), &nonce, &resp) {
        let _ = write_frame(stream, &FrameHeader::new(MsgType::HelloErr, rand::random(), 1), &[0x01]).await;
        anyhow::bail!("HMAC verification failed for {}", hello.agent_name);
    }

    // Issue session token
    let token = {
        let mut b = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut b);
        b.iter().map(|x| format!("{:02x}", x)).collect::<String>()
    };
    let agent_id = db::get_agent_id(pool, &hello.agent_name).await?
        .ok_or_else(|| anyhow::anyhow!("agent id missing"))?;

    // Guard against u64 → i64 overflow in session TTL conversion
    let session_ttl_i64 = i64::try_from(session_ttl)
        .map_err(|_| anyhow::anyhow!("session_ttl_seconds {} exceeds i64::MAX", session_ttl))?;

    // Persist session to DB; then attempt cache operations with compensating
    // cleanup on failure so a DB session is never left without a cache entry.
    db::create_session(pool, &token, agent_id, session_ttl_i64).await?;
    db::touch_agent(pool, agent_id).await?;

    // Scope the Redis lock so it is released before any compensating DB calls.
    let (cache_err, session_cached) = {
        let mut r = redis.lock().await;
        let session_result = cache::cache_session(&mut r, &token, agent_id, session_ttl).await;
        let presence_result = if session_result.is_ok() {
            cache::set_presence(&mut r, agent_id).await
        } else {
            Ok(())
        };
        let session_cached = session_result.is_ok();
        (session_result.and(presence_result).err(), session_cached)
    };
    if let Some(e) = cache_err {
        // Compensate: remove dangling DB session
        let _ = db::delete_session(pool, &token).await;
        // If session was cached before presence write failed, also evict the Redis session
        if session_cached {
            let mut r = redis.lock().await;
            let _ = cache::del_session(&mut r, &token).await;
        }
        return Err(e);
    }

    // Send HELLO_ACK
    let tok_bytes = token.as_bytes();
    let mut ack = Vec::with_capacity(2 + tok_bytes.len() + 8);
    ack.extend_from_slice(&(tok_bytes.len() as u16).to_be_bytes());
    ack.extend_from_slice(tok_bytes);
    ack.extend_from_slice(&agent_id.to_be_bytes());
    if let Err(e) = write_frame(stream, &FrameHeader::new(MsgType::HelloAck, rand::random(), ack.len() as u32), &ack).await {
        // HELLO_ACK failed to send — clean up DB session, Redis session, and presence to avoid
        // orphaned auth state and phantom "online" status
        let _ = db::delete_session(pool, &token).await;
        {
            let mut r = redis.lock().await;
            let _ = cache::del_session(&mut r, &token).await;
            let _ = cache::remove_presence(&mut r, agent_id).await;
        }
        return Err(e.into());
    }

    tracing::info!("Auth OK: {} (id={})", hello.agent_name, agent_id);
    Ok(AuthenticatedAgent { agent_id, agent_name: hello.agent_name, session_token: token })
}
