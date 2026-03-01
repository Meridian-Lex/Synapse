use anyhow::Result;
use sqlx::PgPool;

pub async fn connect(url: &str) -> Result<PgPool> {
    let pool = PgPool::connect(url).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    tracing::info!("Postgres connected, migrations applied");
    Ok(pool)
}

pub async fn get_agent_secret(pool: &PgPool, name: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT secret_hash FROM agents WHERE name = $1"
    ).bind(name).fetch_optional(pool).await?;
    Ok(row.map(|(s,)| s))
}

pub async fn get_agent_id(pool: &PgPool, name: &str) -> Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM agents WHERE name = $1"
    ).bind(name).fetch_optional(pool).await?;
    Ok(row.map(|(id,)| id))
}

pub async fn touch_agent(pool: &PgPool, id: i64) -> Result<()> {
    sqlx::query("UPDATE agents SET last_seen = now() WHERE id = $1")
        .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn get_channel_id(pool: &PgPool, name: &str) -> Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM channels WHERE name = $1 AND archived_at IS NULL"
    ).bind(name).fetch_optional(pool).await?;
    Ok(row.map(|(id,)| id))
}

#[allow(clippy::too_many_arguments)]  // all args are required distinct message fields
pub async fn store_message(
    pool: &PgPool, uuid: i64, channel_id: i64, sender_id: i64,
    content_type: i16, body: &[u8], compressed: bool, priority: i16,
    reply_to: Option<i64>,
) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(
        r#"INSERT INTO messages
           (message_uuid, channel_id, sender_id, content_type, body, compressed, priority, reply_to)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id"#,
    ).bind(uuid).bind(channel_id).bind(sender_id).bind(content_type)
     .bind(body).bind(compressed).bind(priority).bind(reply_to)
     .fetch_one(pool).await?;
    Ok(row.0)
}

pub async fn create_session(pool: &PgPool, token: &str, agent_id: i64, ttl: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO sessions (token, agent_id, expires_at) \
         VALUES ($1, $2, now() + ($3 || ' seconds')::interval)",
    ).bind(token).bind(agent_id).bind(ttl.to_string())
     .execute(pool).await?;
    Ok(())
}

pub async fn delete_session(pool: &PgPool, token: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token = $1")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(())
}
