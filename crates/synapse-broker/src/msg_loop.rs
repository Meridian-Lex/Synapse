use crate::{cache, connection::AuthenticatedAgent, db, router::Router};
use anyhow::Result;
use sqlx::PgPool;
use std::sync::Arc;
use synapse_proto::{
    codec::{read_frame, write_frame},
    compression::{compress, should_compress},
    frame::{Encoding, FrameHeader, MsgType},
    message::MsgPayload,
};
use tokio::{io::{AsyncRead, AsyncWrite}, sync::Mutex, time::{interval, Duration}};
use redis::aio::MultiplexedConnection;

pub async fn run<S>(
    stream: &mut S,
    agent: &AuthenticatedAgent,
    pool: &PgPool,
    redis: &Arc<Mutex<MultiplexedConnection>>,
    router: &Router,
) -> Result<()>
where S: AsyncRead + AsyncWrite + Unpin,
{
    let mut ticker = interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            result = read_frame(stream) => {
                let (hdr, payload) = result?;
                match hdr.msg_type {
                    MsgType::Ping => {
                        write_frame(stream, &FrameHeader::new(MsgType::Pong, hdr.message_id, 0), &[]).await?;
                    }
                    MsgType::Subscribe => {
                        let name = String::from_utf8_lossy(&payload).to_string();
                        if let Some(cid) = db::get_channel_id(pool, &name).await? {
                            router.subscribe(cid).await;
                            tracing::info!("{} subscribed to {}", agent.agent_name, name);
                        }
                    }
                    MsgType::Msg => {
                        let msg = MsgPayload::decode(&payload)?;
                        let (channel_id, content_type) = match &msg {
                            MsgPayload::Dialogue { channel_id, .. } => (*channel_id as i64, 1i16),
                            MsgPayload::Work     { channel_id, .. } => (*channel_id as i64, 2i16),
                        };
                        let (body, compressed, enc) = if should_compress(&payload) {
                            (compress(&payload)?, true, Encoding::Zstd)
                        } else {
                            (payload.clone(), false, Encoding::Raw)
                        };
                        db::store_message(pool, hdr.message_id as i64, channel_id, agent.agent_id,
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
            _ = ticker.tick() => {
                let mut r = redis.lock().await;
                cache::set_presence(&mut r, agent.agent_id).await?;
            }
        }
    }

    let mut r = redis.lock().await;
    cache::remove_presence(&mut r, agent.agent_id).await?;
    Ok(())
}
