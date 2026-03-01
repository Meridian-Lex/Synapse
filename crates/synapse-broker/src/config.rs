use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BrokerConfig {
    pub broker:     BrokerSection,
    pub postgres:   PostgresSection,
    pub redis:      RedisSection,
    pub webui:      WebuiSection,
    pub rate_limit: RateLimitSection,
}

#[derive(Debug, Deserialize)]
pub struct BrokerSection {
    pub listen:              String,
    pub tls_cert:            String,
    pub tls_key:             String,
    pub session_ttl_seconds: u64,
    pub max_frame_bytes:     u32,
}

#[derive(Debug, Deserialize)]
pub struct PostgresSection { pub url: String }

#[derive(Debug, Deserialize)]
pub struct RedisSection { pub url: String }

#[derive(Debug, Deserialize)]
pub struct WebuiSection { pub enabled: bool, pub listen: String, pub read_only: bool }

#[derive(Debug, Deserialize)]
pub struct RateLimitSection { pub messages_per_minute: u32 }

pub fn load(path: &str) -> anyhow::Result<BrokerConfig> {
    Ok(config::Config::builder()
        .add_source(config::File::with_name(path))
        .build()?
        .try_deserialize()?)
}
