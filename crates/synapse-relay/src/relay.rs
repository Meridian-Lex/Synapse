use crate::broker::{BrokerError, TlsBrokerClient};
use crate::buffer::ChannelBuffer;
use crate::config::Config;
use crate::types::{BufferedMessage, ChannelStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    Connected,
    Reconnecting,
    AuthFailed,
    #[allow(dead_code)]
    Disconnected,
}

struct ChannelEntry {
    buffer:     Arc<Mutex<ChannelBuffer>>,
    channel_id: u64,
    conn_state: ConnState,
}

pub struct RelayRegistry {
    inner:  Arc<Mutex<Inner>>,
    config: Arc<Config>,
}

struct Inner {
    channels: HashMap<String, ChannelEntry>,
}

fn expand_tilde(path: &str) -> String {
    path.replace('~', &std::env::var("HOME").unwrap_or_default())
}

impl RelayRegistry {
    pub fn new(config: Config) -> Self {
        Self {
            inner:  Arc::new(Mutex::new(Inner { channels: HashMap::new() })),
            config: Arc::new(config),
        }
    }

    /// Subscribe to a channel. Idempotent.
    pub async fn subscribe(&self, channel: &str) -> anyhow::Result<()> {
        // Check if already subscribed
        {
            let inner = self.inner.lock().unwrap();
            if let Some(e) = inner.channels.get(channel) {
                if e.conn_state != ConnState::AuthFailed {
                    return Ok(());
                }
                anyhow::bail!("channel {} auth failed — restart relay to retry", channel);
            }
        }

        let buffer = Arc::new(Mutex::new(ChannelBuffer::new(self.config.buffer_capacity)));
        let addr = format!("{}:{}", self.config.broker_host, self.config.broker_port);
        let ca_path = self.ca_path();

        // Connect and subscribe
        let mut client = match TlsBrokerClient::connect(&addr, &ca_path, &self.config.agent_name, &self.config.secret).await {
            Ok(c) => c,
            Err(e) => {
                if is_auth_fail(&e) {
                    let mut inner = self.inner.lock().unwrap();
                    inner.channels.insert(channel.to_string(), ChannelEntry {
                        buffer: buffer.clone(),
                        channel_id: 0,
                        conn_state: ConnState::AuthFailed,
                    });
                    anyhow::bail!("auth failed for channel {channel}: {e}");
                }
                return Err(e);
            }
        };

        let channel_id = client.subscribe(channel).await?;

        {
            let mut inner = self.inner.lock().unwrap();
            inner.channels.insert(channel.to_string(), ChannelEntry {
                buffer: buffer.clone(),
                channel_id,
                conn_state: ConnState::Connected,
            });
        }

        // Spawn the message pump
        let channel_name = channel.to_string();
        let registry = self.inner.clone();
        let cfg = self.config.clone();
        tokio::spawn(async move {
            run_pump(channel_name, client, buffer, registry, cfg).await;
        });

        Ok(())
    }

    /// Unsubscribe from a channel.
    pub fn leave(&self, channel: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.channels.remove(channel);
    }

    /// Send a Dialogue message to a channel. Auto-subscribes if needed.
    pub async fn send_dialogue(&self, channel: &str, text: &str) -> anyhow::Result<()> {
        self.ensure_subscribed(channel).await?;
        let channel_id = self.channel_id(channel)?;
        // We can't hold a lock across await, so we need a separate connection for sends.
        // For v1: create a short-lived send connection.
        let mut sender = self.new_send_client().await?;
        sender.subscribe(channel).await?;
        sender.send_dialogue(channel_id, text).await?;
        Ok(())
    }

    /// Send a Work message to a channel.
    pub async fn send_work(&self, channel: &str, value: serde_json::Value) -> anyhow::Result<()> {
        self.ensure_subscribed(channel).await?;
        let channel_id = self.channel_id(channel)?;
        let mut sender = self.new_send_client().await?;
        sender.subscribe(channel).await?;
        sender.send_work(channel_id, value).await?;
        Ok(())
    }

    /// Poll for messages since a given seq.
    pub fn poll(&self, channel: &str, since: u64) -> Vec<BufferedMessage> {
        let inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.channels.get(channel) {
            entry.buffer.lock().unwrap().drain_since(since)
        } else {
            vec![]
        }
    }

    /// Wait for min messages since since, with timeout. Returns (messages, timed_out).
    pub async fn wait(&self, channel: &str, since: u64, min: usize, timeout_ms: u64) -> anyhow::Result<(Vec<BufferedMessage>, bool)> {
        // Ensure subscribed before waiting
        self.ensure_subscribed(channel).await?;
        let buffer = {
            let inner = self.inner.lock().unwrap();
            inner.channels.get(channel).map(|e| e.buffer.clone())
        };
        if let Some(buf) = buffer {
            let msgs = ChannelBuffer::wait_for(buf, since, min, timeout_ms).await;
            let timed_out = msgs.len() < min;
            Ok((msgs, timed_out))
        } else {
            Ok((vec![], true))
        }
    }

    /// List all channels on the broker.
    pub async fn list_channels(&self) -> anyhow::Result<Vec<String>> {
        let mut client = self.new_send_client().await?;
        client.list_channels().await
    }

    /// List users in a channel.
    pub async fn list_users(&self, channel: &str) -> anyhow::Result<Vec<String>> {
        let mut client = self.new_send_client().await?;
        client.list_users(channel).await
    }

    /// Return status of all subscribed channels.
    pub fn status(&self) -> Vec<ChannelStatus> {
        let inner = self.inner.lock().unwrap();
        inner.channels.iter().map(|(name, entry)| ChannelStatus {
            channel: name.clone(),
            connected: entry.conn_state == ConnState::Connected,
            buffered: entry.buffer.lock().unwrap().len(),
        }).collect()
    }

    // --- helpers ---

    async fn ensure_subscribed(&self, channel: &str) -> anyhow::Result<()> {
        let state = self.inner.lock().unwrap()
            .channels.get(channel)
            .map(|e| e.conn_state.clone());
        match state {
            Some(ConnState::AuthFailed) => {
                anyhow::bail!("channel {} auth failed — restart relay to retry", channel);
            }
            Some(_) => Ok(()), // already subscribed and healthy
            None => self.subscribe(channel).await,
        }
    }

    fn channel_id(&self, channel: &str) -> anyhow::Result<u64> {
        self.inner.lock().unwrap()
            .channels.get(channel)
            .map(|e| e.channel_id)
            .ok_or_else(|| anyhow::anyhow!("channel {channel} not subscribed"))
    }

    fn broker_addr(&self) -> String {
        format!("{}:{}", self.config.broker_host, self.config.broker_port)
    }

    fn ca_path(&self) -> String {
        expand_tilde(
            self.config.credentials.credentials_file
                .as_deref()
                .unwrap_or("~/.config/synapse/credentials.toml")
        )
    }

    async fn new_send_client(&self) -> anyhow::Result<TlsBrokerClient> {
        TlsBrokerClient::connect(
            &self.broker_addr(),
            &self.ca_path(),
            &self.config.agent_name,
            &self.config.secret,
        ).await
    }
}

/// Message pump: reads messages from broker and pushes to buffer. Reconnects on disconnect.
async fn run_pump(
    channel: String,
    mut client: TlsBrokerClient,
    buffer: Arc<Mutex<ChannelBuffer>>,
    registry: Arc<Mutex<Inner>>,
    config: Arc<Config>,
) {
    let addr = format!("{}:{}", config.broker_host, config.broker_port);
    let ca_path = expand_tilde(
        config.credentials.credentials_file
            .as_deref()
            .unwrap_or("~/.config/synapse/credentials.toml")
    );
    let mut backoff_secs = config.reconnect_delay_secs;

    loop {
        // Drain messages
        loop {
            match client.read_message().await {
                Ok(Some(msg)) => {
                    let mut buf = buffer.lock().unwrap();
                    match msg.msg_type {
                        crate::types::MessageType::Dialogue => {
                            buf.push_dialogue(msg.text.unwrap_or_default());
                        }
                        crate::types::MessageType::Work => {
                            buf.push_work(msg.payload.unwrap_or(serde_json::Value::Null));
                        }
                    }
                }
                Ok(None) => {
                    info!("broker disconnected channel {channel}");
                    break;
                }
                Err(e) => {
                    error!("broker error on channel {channel}: {e}");
                    break;
                }
            }
        }

        // Check if channel was left while pumping
        {
            let inner = registry.lock().unwrap();
            if !inner.channels.contains_key(&channel) {
                info!("channel {channel} removed — pump exiting");
                return;
            }
        }

        // Mark as reconnecting
        {
            let mut inner = registry.lock().unwrap();
            if let Some(entry) = inner.channels.get_mut(&channel) {
                entry.conn_state = ConnState::Reconnecting;
            }
        }

        // Reconnect with exponential backoff
        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(config.reconnect_max_secs);

        match TlsBrokerClient::connect(&addr, &ca_path, &config.agent_name, &config.secret).await {
            Ok(mut new_client) => {
                match new_client.subscribe(&channel).await {
                    Ok(channel_id) => {
                        {
                            let mut inner = registry.lock().unwrap();
                            if let Some(entry) = inner.channels.get_mut(&channel) {
                                entry.conn_state = ConnState::Connected;
                                entry.channel_id = channel_id;
                            }
                        }
                        client = new_client;
                        backoff_secs = config.reconnect_delay_secs; // reset on success
                        info!("reconnected channel {channel}");
                    }
                    Err(e) => {
                        error!("failed to re-subscribe {channel}: {e}");
                    }
                }
            }
            Err(e) => {
                if is_auth_fail(&e) {
                    error!("auth failed on reconnect for {channel} — stopping pump");
                    let mut inner = registry.lock().unwrap();
                    if let Some(entry) = inner.channels.get_mut(&channel) {
                        entry.conn_state = ConnState::AuthFailed;
                    }
                    return;
                }
                warn!("reconnect failed for {channel}: {e} — will retry");
            }
        }
    }
}

fn is_auth_fail(e: &anyhow::Error) -> bool {
    e.downcast_ref::<BrokerError>()
        .map(|be| matches!(be, BrokerError::AuthFailed))
        .unwrap_or(false)
}
