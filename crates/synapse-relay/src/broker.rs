use anyhow::{Context, Result};
use rustls::ClientConfig;
use rustls_pemfile::certs;
use std::{fs::File, io::BufReader, sync::Arc};
use tokio::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{client::TlsStream, TlsConnector};
use synapse_proto::{
    auth::{compute_hmac, HelloPayload},
    codec::{read_frame, write_frame},
    compression::{compress, decompress, should_compress},
    frame::{Encoding, FrameHeader, MsgType},
    message::MsgPayload,
};
use crate::types::MessageType;

/// An inbound message received from the broker.
#[derive(Debug, Clone)]
pub struct InboundMsg {
    #[allow(dead_code)]
    pub channel_id: u64,
    pub msg_type:   MessageType,
    pub text:       Option<String>,
    pub payload:    Option<serde_json::Value>,
}

/// Type alias for the standard TLS-based broker stream.
pub type TlsBrokerClient = BrokerClient<TlsStream<TcpStream>>;

/// Error types specific to broker operations.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("authentication failed")]
    AuthFailed,
    #[allow(dead_code)]
    #[error("broker disconnected")]
    Disconnected,
    #[allow(dead_code)]
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// A connected, authenticated broker client. Generic over the stream type.
pub struct BrokerClient<S> {
    stream: S,
    #[allow(dead_code)]
    agent_id: i64,
}

/// Generic implementation for all stream types.
impl<S: AsyncRead + AsyncWrite + Unpin + Send> BrokerClient<S> {
    #[allow(dead_code)]
    pub fn agent_id(&self) -> i64 { self.agent_id }

    /// Subscribe to a channel and return the broker-assigned channel_id.
    pub async fn subscribe(&mut self, channel: &str) -> Result<u64> {
        let payload = channel.as_bytes().to_vec();
        write_frame(&mut self.stream, &FrameHeader::new(MsgType::Subscribe, rand::random(), payload.len() as u32), &payload).await?;
        let (ack, ack_payload) = tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            read_frame(&mut self.stream),
        ).await.map_err(|_| anyhow::anyhow!("timeout waiting for SubscribeAck"))??;
        if ack.msg_type == MsgType::Error {
            anyhow::bail!("broker error: {}", String::from_utf8_lossy(&ack_payload));
        }
        anyhow::ensure!(ack.msg_type == MsgType::SubscribeAck, "expected SubscribeAck, got {:?}", ack.msg_type);
        anyhow::ensure!(ack_payload.len() == 8, "SubscribeAck payload wrong length");
        Ok(u64::from_be_bytes(ack_payload.try_into().unwrap()))
    }

    /// Send a Dialogue message to a channel.
    pub async fn send_dialogue(&mut self, channel_id: u64, text: &str) -> Result<()> {
        let ts = now_ms();
        let mut payload = MsgPayload::Dialogue { channel_id, timestamp_ms: ts, body: text.into() }.encode()?;
        let mut hdr = FrameHeader::new(MsgType::Msg, rand::random(), payload.len() as u32);
        if should_compress(&payload) {
            payload = compress(&payload)?;
            hdr.flags.compressed = true;
            hdr.encoding = Encoding::Zstd;
            hdr.payload_len = payload.len() as u32;
        }
        write_frame(&mut self.stream, &hdr, &payload).await?;
        Ok(())
    }

    /// Send a Work message to a channel. JSON value is serialised to MessagePack.
    pub async fn send_work(&mut self, channel_id: u64, value: serde_json::Value) -> Result<()> {
        let ts = now_ms();
        let rmpv_val = json_to_rmpv(value);
        let mut payload = MsgPayload::Work { channel_id, timestamp_ms: ts, body: rmpv_val }.encode()?;
        let mut hdr = FrameHeader::new(MsgType::Msg, rand::random(), payload.len() as u32);
        if should_compress(&payload) {
            payload = compress(&payload)?;
            hdr.flags.compressed = true;
            hdr.encoding = Encoding::Zstd;
            hdr.payload_len = payload.len() as u32;
        }
        write_frame(&mut self.stream, &hdr, &payload).await?;
        Ok(())
    }

    /// Read the next inbound message. Handles Ping/Pong transparently.
    /// Returns `None` on `Bye` or clean disconnect.
    pub async fn read_message(&mut self) -> Result<Option<InboundMsg>> {
        loop {
            let (hdr, payload) = match read_frame(&mut self.stream).await {
                Ok(f) => f,
                Err(e) if is_eof(&e) => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            match hdr.msg_type {
                MsgType::Ping => {
                    write_frame(&mut self.stream, &FrameHeader::new(MsgType::Pong, hdr.message_id, 0), &[]).await?;
                    continue;
                }
                MsgType::Bye => return Ok(None),
                MsgType::Msg => {
                    let data = if hdr.encoding == Encoding::Zstd {
                        decompress(&payload)?
                    } else {
                        payload
                    };
                    let msg_payload = synapse_proto::message::MsgPayload::decode(&data)?;
                    let inbound = match msg_payload {
                        MsgPayload::Dialogue { channel_id, body, .. } => InboundMsg {
                            channel_id,
                            msg_type: MessageType::Dialogue,
                            text: Some(body),
                            payload: None,
                        },
                        MsgPayload::Work { channel_id, body, .. } => InboundMsg {
                            channel_id,
                            msg_type: MessageType::Work,
                            text: None,
                            payload: Some(rmpv_to_json(body)),
                        },
                    };
                    return Ok(Some(inbound));
                }
                _ => continue, // ignore other frame types
            }
        }
    }

    /// List all channels on the broker. Sends ChanList, reads response.
    /// Response payload is newline-delimited channel names.
    pub async fn list_channels(&mut self) -> Result<Vec<String>> {
        write_frame(&mut self.stream, &FrameHeader::new(MsgType::ChanList, rand::random(), 0), &[]).await?;
        let (_hdr, payload) = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            read_frame(&mut self.stream),
        ).await.map_err(|_| anyhow::anyhow!("timeout waiting for ChanList response"))??;
        let names = String::from_utf8_lossy(&payload)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        Ok(names)
    }

    /// List users present in a channel. Sends PresenceReq, reads Presence response.
    /// Response payload is newline-delimited agent names.
    pub async fn list_users(&mut self, channel: &str) -> Result<Vec<String>> {
        let payload = channel.as_bytes().to_vec();
        write_frame(&mut self.stream, &FrameHeader::new(MsgType::PresenceReq, rand::random(), payload.len() as u32), &payload).await?;
        let (_hdr, resp_payload) = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            read_frame(&mut self.stream),
        ).await.map_err(|_| anyhow::anyhow!("timeout waiting for Presence response"))??;
        let names = String::from_utf8_lossy(&resp_payload)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        Ok(names)
    }
}

/// TLS-specific implementation with the standard connect method.
impl BrokerClient<TlsStream<TcpStream>> {
    /// Connect to the broker at `addr` (e.g. "localhost:7777"), authenticate, and return a client.
    /// `ca_path`: path to the CA certificate PEM file.
    pub async fn connect(
        addr: &str,
        ca_path: &str,
        agent_name: &str,
        secret: &str,
    ) -> Result<Self> {
        let stream = tls_connect(addr, ca_path).await
            .context("TLS connect failed")?;
        let (agent_id, stream) = authenticate(stream, agent_name, secret).await
            .context("authentication failed")?;
        Ok(Self { stream, agent_id })
    }
}

/// Plain TCP implementation for testing (cfg(test) only).
#[cfg(test)]
impl BrokerClient<TcpStream> {
    /// Connect via plain TCP (no TLS) and authenticate. For testing only.
    #[allow(dead_code)]
    pub async fn connect_plain(
        addr: &str,
        agent_name: &str,
        secret: &str,
    ) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let (agent_id, stream) = authenticate(stream, agent_name, secret).await
            .context("authentication failed")?;
        Ok(Self { stream, agent_id })
    }
}

// --- helpers ---

async fn tls_connect(addr: &str, ca_path: &str) -> Result<TlsStream<TcpStream>> {
    let mut root_store = rustls::RootCertStore::empty();
    let mut cert_count = 0usize;
    for cert in certs(&mut BufReader::new(File::open(ca_path)?)).filter_map(Result::ok) {
        root_store.add(cert)?;
        cert_count += 1;
    }
    anyhow::ensure!(cert_count > 0, "no valid CA certificates found in {ca_path}");
    let config = ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();
    let host = parse_host(addr);
    let stream = TcpStream::connect(addr).await?;
    let server_name = rustls::pki_types::ServerName::try_from(host)?;
    Ok(TlsConnector::from(Arc::new(config)).connect(server_name, stream).await?)
}

async fn authenticate<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    agent_name: &str,
    secret: &str,
) -> Result<(i64, S)> {
    let hello = HelloPayload {
        agent_name: agent_name.into(),
        client_version: format!("synapse-relay/{}", env!("CARGO_PKG_VERSION")),
        capabilities: 0,
    };
    let payload = hello.encode()?;
    write_frame(&mut stream, &FrameHeader::new(MsgType::Hello, rand::random(), payload.len() as u32), &payload).await?;

    let (ch, nonce_bytes) = read_frame(&mut stream).await?;
    if ch.msg_type == MsgType::HelloErr {
        return Err(BrokerError::AuthFailed.into());
    }
    anyhow::ensure!(ch.msg_type == MsgType::Challenge && nonce_bytes.len() == 32, "unexpected auth response");
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&nonce_bytes);

    let resp = compute_hmac(secret.as_bytes(), &nonce);
    write_frame(&mut stream, &FrameHeader::new(MsgType::HelloResp, rand::random(), resp.len() as u32), &resp).await?;

    let (ack, ack_payload) = read_frame(&mut stream).await?;
    if ack.msg_type == MsgType::HelloErr {
        return Err(BrokerError::AuthFailed.into());
    }
    anyhow::ensure!(ack.msg_type == MsgType::HelloAck, "auth rejected by broker");

    let tl = u16::from_be_bytes([ack_payload[0], ack_payload[1]]) as usize;
    anyhow::ensure!(ack_payload.len() >= 2 + tl + 8, "HelloAck payload truncated");
    let agent_id = i64::from_be_bytes(ack_payload[2 + tl..2 + tl + 8].try_into()?);
    Ok((agent_id, stream))
}

fn parse_host(addr: &str) -> String {
    if addr.starts_with('[') {
        addr.trim_start_matches('[').split(']').next().unwrap_or(addr).to_string()
    } else {
        addr.split(':').next().unwrap_or(addr).to_string()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn is_eof(e: &synapse_proto::error::ProtoError) -> bool {
    matches!(e, synapse_proto::error::ProtoError::Io(io_err) if io_err.kind() == std::io::ErrorKind::UnexpectedEof)
}

/// Convert serde_json::Value to rmpv::Value for sending Work frames.
fn json_to_rmpv(v: serde_json::Value) -> rmpv::Value {
    match v {
        serde_json::Value::Null => rmpv::Value::Nil,
        serde_json::Value::Bool(b) => rmpv::Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { rmpv::Value::Integer(i.into()) }
            else if let Some(u) = n.as_u64() { rmpv::Value::Integer(u.into()) }
            else { rmpv::Value::F64(n.as_f64().unwrap_or(f64::NAN)) }
        }
        serde_json::Value::String(s) => rmpv::Value::String(s.into()),
        serde_json::Value::Array(a) => rmpv::Value::Array(a.into_iter().map(json_to_rmpv).collect()),
        serde_json::Value::Object(m) => rmpv::Value::Map(
            m.into_iter().map(|(k, v)| (rmpv::Value::String(k.into()), json_to_rmpv(v))).collect()
        ),
    }
}

/// Convert rmpv::Value to serde_json::Value for serving Work frames via HTTP.
fn rmpv_to_json(v: rmpv::Value) -> serde_json::Value {
    match v {
        rmpv::Value::Nil => serde_json::Value::Null,
        rmpv::Value::Boolean(b) => serde_json::Value::Bool(b),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() { serde_json::json!(n) }
            else if let Some(n) = i.as_u64() { serde_json::json!(n) }
            else { serde_json::Value::Null }
        }
        rmpv::Value::F32(f) => serde_json::json!(f as f64),
        rmpv::Value::F64(f) => serde_json::json!(f),
        rmpv::Value::String(s) => serde_json::Value::String(s.into_str().unwrap_or_default()),
        rmpv::Value::Binary(b) => serde_json::Value::String(base64_encode(&b)),
        rmpv::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(rmpv_to_json).collect()),
        rmpv::Value::Map(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                let key = match k {
                    rmpv::Value::String(s) => s.into_str().unwrap_or_default(),
                    other => format!("{other}"),
                };
                map.insert(key, rmpv_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        rmpv::Value::Ext(_, _) => serde_json::Value::Null,
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        out.push(CHARS[b0 >> 2] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        out.push(if chunk.len() > 1 { CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[b2 & 0x3f] as char } else { '=' });
    }
    out
}
