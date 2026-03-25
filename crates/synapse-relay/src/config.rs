use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Credentials {
    pub credentials_file: Option<String>,
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            credentials_file: Some("~/.config/synapse/credentials.toml".into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]            pub bind: String,
    #[serde(default = "default_port")]            pub port: u16,
    #[serde(default = "default_broker_host")]     pub broker_host: String,
    #[serde(default = "default_broker_port")]     pub broker_port: u16,
    #[serde(default = "default_capacity")]        pub buffer_capacity: usize,
    #[serde(default = "default_reconnect_delay")] pub reconnect_delay_secs: u64,
    #[serde(default = "default_reconnect_max")]   pub reconnect_max_secs: u64,
    #[serde(default)]                             pub credentials: Credentials,
    #[serde(skip)]                                pub agent_name: String,
    #[serde(skip)]                                pub secret: String,
}

fn default_bind()            -> String { "127.0.0.1".into() }
fn default_port()            -> u16    { 7779 }
fn default_broker_host()     -> String { "localhost".into() }
fn default_broker_port()     -> u16    { 7777 }
fn default_capacity()        -> usize  { 1000 }
fn default_reconnect_delay() -> u64    { 2 }
fn default_reconnect_max()   -> u64    { 30 }

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            broker_host: default_broker_host(),
            broker_port: default_broker_port(),
            buffer_capacity: default_capacity(),
            reconnect_delay_secs: default_reconnect_delay(),
            reconnect_max_secs: default_reconnect_max(),
            credentials: Credentials::default(),
            agent_name: String::new(),
            secret: String::new(),
        }
    }
}

impl Config {
    pub fn defaults() -> Self {
        Self::default()
    }

    pub fn parse_toml(s: &str) -> anyhow::Result<Self> {
        let mut cfg: Config = toml::from_str(s)?;
        cfg.agent_name = std::env::var("SYNAPSE_AGENT").unwrap_or_default();
        cfg.secret     = std::env::var("SYNAPSE_SECRET").unwrap_or_default();
        Ok(cfg)
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = config_path();
        let content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        Self::parse_toml(&content)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agent_name.is_empty() {
            anyhow::bail!("SYNAPSE_AGENT env var required");
        }
        if self.secret.is_empty() {
            anyhow::bail!("SYNAPSE_SECRET env var required");
        }
        Ok(())
    }
}

fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".config/synapse-relay/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = Config::defaults();
        assert_eq!(cfg.port, 7779);
        assert_eq!(cfg.bind, "127.0.0.1");
        assert_eq!(cfg.buffer_capacity, 1000);
        assert_eq!(cfg.reconnect_delay_secs, 2);
        assert_eq!(cfg.reconnect_max_secs, 30);
    }

    #[test]
    fn test_from_str_partial() {
        let raw = r#"port = 8888"#;
        let cfg = Config::parse_toml(raw).unwrap();
        assert_eq!(cfg.port, 8888);
        assert_eq!(cfg.bind, "127.0.0.1"); // default preserved
    }

    #[test]
    fn test_validate_success() {
        let cfg = Config {
            agent_name: "test-agent".into(),
            secret: "test-secret".into(),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_missing_agent() {
        let cfg = Config {
            agent_name: String::new(),
            secret: "test-secret".into(),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_missing_secret() {
        let cfg = Config {
            agent_name: "test-agent".into(),
            secret: String::new(),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }
}
