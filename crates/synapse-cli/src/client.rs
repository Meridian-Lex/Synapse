use anyhow::Result;
use rustls::ClientConfig;
use rustls_pemfile::certs;
use std::{fs::File, io::BufReader, sync::Arc};
use tokio::net::TcpStream;
use tokio_rustls::{client::TlsStream, TlsConnector};
use synapse_proto::{
    auth::{compute_hmac, HelloPayload},
    codec::{read_frame, write_frame},
    compression::{compress, should_compress},
    frame::{Encoding, FrameHeader, MsgType},
    message::MsgPayload,
};

pub async fn connect(addr: &str, ca_path: &str) -> Result<TlsStream<TcpStream>> {
    let mut root_store = rustls::RootCertStore::empty();
    for cert in certs(&mut BufReader::new(File::open(ca_path)?)).filter_map(Result::ok) {
        root_store.add(cert)?;
    }
    let config = ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();
    let host = addr.split(':').next().unwrap_or(addr).to_string();
    let stream = TcpStream::connect(addr).await?;
    let server_name = rustls::pki_types::ServerName::try_from(host)?;
    Ok(TlsConnector::from(Arc::new(config)).connect(server_name, stream).await?)
}

pub async fn authenticate<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    name: &str,
    secret: &str,
) -> Result<(i64, String)> {
    let hello = HelloPayload {
        agent_name: name.into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: 0,
    };
    let payload = hello.encode()?;
    write_frame(stream, &FrameHeader::new(MsgType::Hello, rand::random(), payload.len() as u32), &payload).await?;

    let (ch, nonce_bytes) = read_frame(stream).await?;
    anyhow::ensure!(ch.msg_type == MsgType::Challenge && nonce_bytes.len() == 32);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&nonce_bytes);

    let resp = compute_hmac(secret.as_bytes(), &nonce);
    write_frame(stream, &FrameHeader::new(MsgType::HelloResp, rand::random(), resp.len() as u32), &resp).await?;

    let (ack, ack_payload) = read_frame(stream).await?;
    anyhow::ensure!(ack.msg_type == MsgType::HelloAck, "auth rejected by broker");
    let tl = u16::from_be_bytes([ack_payload[0], ack_payload[1]]) as usize;
    let token = String::from_utf8(ack_payload[2..2+tl].to_vec())?;
    let agent_id = i64::from_be_bytes(ack_payload[2+tl..].try_into()?);
    Ok((agent_id, token))
}

pub async fn send_dialogue<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    channel_id: u64,
    text: &str,
) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
    let mut payload = MsgPayload::Dialogue { channel_id, timestamp_ms: ts, body: text.into() }.encode()?;
    let mut hdr = FrameHeader::new(MsgType::Msg, rand::random(), payload.len() as u32);
    if should_compress(&payload) {
        payload = compress(&payload)?;
        hdr.flags.compressed = true;
        hdr.encoding = Encoding::Zstd;
        hdr.payload_len = payload.len() as u32;
    }
    write_frame(stream, &hdr, &payload).await?;
    Ok(())
}
