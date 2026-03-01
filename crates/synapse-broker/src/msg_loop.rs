use crate::{cache, connection::AuthenticatedAgent, db, router::Router};
use anyhow::Result;
use sqlx::PgPool;
use std::sync::Arc;
use synapse_proto::{
    codec::{read_frame, write_frame},
    compression::{compress, decompress, should_compress},
    frame::{Encoding, FrameHeader, MsgType},
    message::MsgPayload,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, Mutex},
    time::{interval, Duration},
};
use redis::aio::MultiplexedConnection;

pub async fn run<S>(
    stream: &mut S,
    agent: &AuthenticatedAgent,
    pool: &PgPool,
    redis: &Arc<Mutex<MultiplexedConnection>>,
    router: &Router,
    max_frame_bytes: u32,
) -> Result<()>
where S: AsyncRead + AsyncWrite + Unpin,
{
    let result = run_inner(stream, agent, pool, redis, router, max_frame_bytes).await;

    // Presence cleanup always runs, even on error exit.
    let mut r = redis.lock().await;
    let _ = cache::remove_presence(&mut r, agent.agent_id).await;

    result
}

async fn run_inner<S>(
    stream: &mut S,
    agent: &AuthenticatedAgent,
    pool: &PgPool,
    redis: &Arc<Mutex<MultiplexedConnection>>,
    router: &Router,
    max_frame_bytes: u32,
) -> Result<()>
where S: AsyncRead + AsyncWrite + Unpin,
{
    let mut ticker = interval(Duration::from_secs(5));

    // Outbound channel: subscription forwarder tasks send raw frames here,
    // the select loop writes them to the client stream.
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    loop {
        tokio::select! {
            // Incoming frame from the connected agent.
            result = read_frame(stream) => {
                let (hdr, payload) = result?;
                // Enforce broker-level max frame size (may be lower than protocol MAX_PAYLOAD).
                if hdr.payload_len > max_frame_bytes {
                    anyhow::bail!("frame size {} exceeds broker max_frame_bytes {}", hdr.payload_len, max_frame_bytes);
                }
                match hdr.msg_type {
                    MsgType::Ping => {
                        write_frame(stream, &FrameHeader::new(MsgType::Pong, hdr.message_id, 0), &[]).await?;
                    }
                    MsgType::Subscribe => {
                        let name = String::from_utf8_lossy(&payload).to_string();
                        if let Some(cid) = db::get_channel_id(pool, &name).await? {
                            // Subscribe and retain the receiver — store it in a
                            // forwarding task rather than dropping it.
                            let mut rx = router.subscribe(cid).await;
                            let tx = outbound_tx.clone();
                            tokio::spawn(async move {
                                loop {
                                    match rx.recv().await {
                                        Ok(frame) => {
                                            if tx.send(frame).is_err() {
                                                // Outbound channel closed; agent disconnected.
                                                break;
                                            }
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                            tracing::warn!(
                                                "broadcast receiver on channel {} lagged, dropped {} messages",
                                                cid, n
                                            );
                                            // Continue — next recv() will return the oldest available.
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                            break;
                                        }
                                    }
                                }
                            });
                            tracing::info!("{} subscribed to {}", agent.agent_name, name);
                            // Reply with resolved channel_id so the client can address
                            // Msg frames to the correct channel without guessing.
                            write_frame(
                                stream,
                                &FrameHeader::new(MsgType::SubscribeAck, hdr.message_id, 8),
                                &cid.to_be_bytes(),
                            ).await?;
                        } else {
                            tracing::warn!("{} subscribe failed: channel not found: {}", agent.agent_name, name);
                            let msg = format!("channel not found: {}", name);
                            write_frame(
                                stream,
                                &FrameHeader::new(MsgType::Error, hdr.message_id, msg.len() as u32),
                                msg.as_bytes(),
                            ).await?;
                        }
                    }
                    MsgType::Msg => {
                        // Decompress before decoding — sender may have compressed large payloads.
                        let decoded = if hdr.encoding == Encoding::Zstd {
                            decompress(&payload)?
                        } else {
                            payload.clone()
                        };
                        let msg = MsgPayload::decode(&decoded)?;
                        let (channel_id, content_type) = match &msg {
                            MsgPayload::Dialogue { channel_id, .. } => (*channel_id as i64, 1i16),
                            MsgPayload::Work     { channel_id, .. } => (*channel_id as i64, 2i16),
                        };
                        // Minor fix: safe u64 -> i64 cast instead of bare `as i64`.
                        let msg_id: i64 = hdr.message_id.try_into().unwrap_or(i64::MAX);
                        // If already compressed by sender, store as-is; otherwise compress if large.
                        let (body, compressed, enc) = if hdr.flags.compressed {
                            (payload.clone(), true, Encoding::Zstd)
                        } else if should_compress(&decoded) {
                            (compress(&decoded)?, true, Encoding::Zstd)
                        } else {
                            (decoded, false, Encoding::Raw)
                        };
                        db::store_message(pool, msg_id, channel_id, agent.agent_id,
                            content_type, &body, compressed, 0, None).await?;
                        let mut route_hdr = FrameHeader::new(MsgType::Msg, hdr.message_id, body.len() as u32);
                        route_hdr.encoding = enc;
                        let mut frame = route_hdr.to_bytes().to_vec();
                        frame.extend_from_slice(&body);
                        router.publish(channel_id, frame.clone()).await;
                        {
                            let mut r = redis.lock().await;
                            cache::publish_message(&mut r, channel_id, &frame).await?;
                        }
                        write_frame(stream, &FrameHeader::new(MsgType::MsgAck, hdr.message_id, 0), &[]).await?;
                    }
                    MsgType::Bye => {
                        tracing::info!("{} disconnected", agent.agent_name);
                        break;
                    }
                    _ => {}
                }
            }

            // Forwarded frames from subscribed broadcast channels.
            Some(frame) = outbound_rx.recv() => {
                // The frame already contains the full serialised header + payload
                // as assembled in the Msg handler above; write it directly.
                stream.write_all(&frame).await?;
                stream.flush().await?;
            }

            // Periodic presence heartbeat.
            _ = ticker.tick() => {
                let mut r = redis.lock().await;
                cache::set_presence(&mut r, agent.agent_id).await?;
            }
        }
    }

    Ok(())
}
