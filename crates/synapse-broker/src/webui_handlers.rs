use rand::Rng;
use serde::Serialize;
use sqlx::PgPool;
use synapse_proto::compression::decompress;
use synapse_proto::frame::{Encoding, FrameHeader, MsgType, HEADER_LEN};

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
               COALESCE(f.name, '') AS "fleet_name!"
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
        fleet_name:         if row.fleet_name.is_empty() { None } else { Some(row.fleet_name) },
        default_channel_id: row.default_channel_id,
        agent_uuid:         row.agent_uuid,
    })
}

/// All channels visible to the given fleet (own + shared).
pub async fn fetch_channel_list(pool: &PgPool, fleet_id: i64) -> Vec<ChannelInfo> {
    let rows = sqlx::query!(
        r#"
        SELECT c.id, c.name AS "name!", f.name AS "fleet_name!"
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
    .unwrap_or_default();

    rows.into_iter()
        .map(|r| ChannelInfo { id: r.id, name: r.name, fleet_name: r.fleet_name })
        .collect()
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

/// Construct and publish a Dialogue frame as a human operator.
pub async fn send_as_human(
    pool:       &PgPool,
    router:     &crate::router::Router,
    ctx:        &AgentContext,
    channel_id: i64,
    body:       &str,
) -> Result<(), anyhow::Error> {
    // Payload: 0x01 (Dialogue content_type) + 16-byte agent UUID + UTF-8 body
    let uuid_bytes = ctx.agent_uuid.as_bytes();
    let mut payload = Vec::with_capacity(1 + 16 + body.len());
    payload.push(0x01u8);
    payload.extend_from_slice(uuid_bytes);
    payload.extend_from_slice(body.as_bytes());

    // message_id for frame header is a random u64
    let message_id: u64 = rand::thread_rng().gen();
    // message_uuid in DB is BIGINT — use lower 63 bits to stay positive
    let message_uuid: i64 = (message_id & 0x7FFF_FFFF_FFFF_FFFF) as i64;

    let hdr = FrameHeader::new(MsgType::Msg, message_id, payload.len() as u32);

    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&hdr.to_bytes());
    frame.extend_from_slice(&payload);

    // Persist to DB
    sqlx::query!(
        r#"INSERT INTO messages
           (message_uuid, channel_id, sender_id, content_type, body, compressed, priority)
           VALUES ($1, $2, $3, $4, $5, false, $6)"#,
        message_uuid,
        channel_id,
        ctx.agent_id,
        1i16,
        body.as_bytes(),
        0i16
    )
    .execute(pool)
    .await?;

    // Publish to all channel subscribers
    router.publish(channel_id, frame).await;

    Ok(())
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
        RETURNING id, name AS "name!"
        "#,
        name,
        description,
        fleet_id,
        creator_id
    )
    .fetch_one(pool)
    .await?;

    let fleet_name = sqlx::query_scalar!(
        r#"SELECT name AS "name!" FROM fleets WHERE id = $1"#,
        fleet_id
    )
    .fetch_one(pool)
    .await?;

    Ok(ChannelInfo { id: row.id, name: row.name, fleet_name })
}
