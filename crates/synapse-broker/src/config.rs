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
#[allow(dead_code)]  // read_only is parsed from config; reserved for future access-control enforcement
pub struct WebuiSection { pub enabled: bool, pub listen: String, pub read_only: bool }

#[derive(Debug, Deserialize)]
pub struct RateLimitSection { pub messages_per_minute: u32 }

pub fn load(path: &str) -> anyhow::Result<BrokerConfig> {
    let cfg: BrokerConfig = config::Config::builder()
        .add_source(config::File::with_name(path))
        .build()?
        .try_deserialize()?;

    if cfg.broker.session_ttl_seconds == 0 {
        return Err(anyhow::anyhow!("broker.session_ttl_seconds must be > 0"));
    }
    if cfg.broker.max_frame_bytes == 0 {
        return Err(anyhow::anyhow!("broker.max_frame_bytes must be > 0"));
    }
    if cfg.rate_limit.messages_per_minute == 0 {
        return Err(anyhow::anyhow!("rate_limit.messages_per_minute must be > 0"));
    }

    if !std::path::Path::new(&cfg.broker.tls_cert).exists() {
        anyhow::bail!("broker.tls_cert path does not exist: {}", cfg.broker.tls_cert);
    }
    if !std::path::Path::new(&cfg.broker.tls_key).exists() {
        anyhow::bail!("broker.tls_key path does not exist: {}", cfg.broker.tls_key);
    }

    Ok(cfg)
}
