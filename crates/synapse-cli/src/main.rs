mod client;

use anyhow::Result;
use clap::{Parser, Subcommand};
use synapse_proto::{codec::{read_frame, write_frame}, frame::{Encoding, FrameHeader, MsgType}};

#[derive(Parser)]
#[command(name = "synapse", about = "Synapse fleet communications client")]
struct Cli {
    #[arg(long, env = "SYNAPSE_HOST",   default_value = "localhost:7777")] host:   String,
    #[arg(long, env = "SYNAPSE_CA",     default_value = "/etc/synapse/ca.pem")] ca: String,
    #[arg(long, env = "SYNAPSE_AGENT")]  agent:  String,
    #[arg(long, env = "SYNAPSE_SECRET")] secret: String,
    #[command(subcommand)] cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Send {
        /// Channel name (e.g. #general) or numeric ID
        #[arg(long, default_value = "#general")] channel: String,
        message: String,
    },
    Listen {
        /// Channel name (e.g. #general) or numeric ID
        #[arg(long, default_value = "#general")] channel: String,
    },
}

/// Send a SUBSCRIBE frame and return the broker-resolved channel ID via SubscribeAck.
async fn subscribe_channel<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    channel: &str,
) -> Result<u64> {
    let payload = channel.as_bytes().to_vec();
    write_frame(stream, &FrameHeader::new(MsgType::Subscribe, rand::random(), payload.len() as u32), &payload).await?;
    // Read SubscribeAck — broker replies with the resolved channel_id (8 bytes BE).
    // 10-second timeout guards against broker silence (e.g. network partition or unknown channel).
    let (ack, ack_payload) = tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        read_frame(stream),
    ).await
    .map_err(|_| anyhow::anyhow!("timed out waiting for SubscribeAck on channel '{}'", channel))??;
    if ack.msg_type == MsgType::Error {
        anyhow::bail!("broker error: {}", String::from_utf8_lossy(&ack_payload));
    }
    anyhow::ensure!(ack.msg_type == MsgType::SubscribeAck, "expected SubscribeAck, got {:?}", ack.msg_type);
    anyhow::ensure!(ack_payload.len() == 8, "SubscribeAck payload wrong length");
    Ok(u64::from_be_bytes(ack_payload.try_into().unwrap()))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let mut stream = client::connect(&cli.host, &cli.ca).await?;
    client::authenticate(&mut stream, &cli.agent, &cli.secret).await?;

    match cli.cmd {
        Cmd::Send { channel, message } => {
            let channel_id = subscribe_channel(&mut stream, &channel).await?;
            client::send_dialogue(&mut stream, channel_id, &message).await?;
            let (ack, _) = read_frame(&mut stream).await?;
            if ack.msg_type == MsgType::MsgAck { println!("Delivered."); }
        }
        Cmd::Listen { channel } => {
            subscribe_channel(&mut stream, &channel).await?;
            println!("Listening on {} ... (Ctrl+C to stop)", channel);
            loop {
                let (hdr, payload) = read_frame(&mut stream).await?;
                match hdr.msg_type {
                    MsgType::Msg if payload.len() > 17 && payload[0] == 0x01 => {
                        let data = if hdr.encoding == Encoding::Zstd {
                            match synapse_proto::compression::decompress(&payload) {
                                Ok(d) => d,
                                Err(e) => {
                                    eprintln!("Decompression failed: {e}");
                                    continue;
                                }
                            }
                        } else {
                            payload
                        };
                        if data.len() > 17 && data[0] == 0x01 {
                            println!("{}", String::from_utf8_lossy(&data[17..]));
                        }
                    }
                    MsgType::Ping => {
                        write_frame(&mut stream, &FrameHeader::new(MsgType::Pong, hdr.message_id, 0), &[]).await?;
                    }
                    MsgType::Bye => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
