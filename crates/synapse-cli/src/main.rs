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
        #[arg(long, default_value = "#general")] channel: String,
        message: String,
    },
    Listen {
        #[arg(long, default_value = "#general")] channel: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let mut stream = client::connect(&cli.host, &cli.ca).await?;
    client::authenticate(&mut stream, &cli.agent, &cli.secret).await?;

    match cli.cmd {
        Cmd::Send { channel: _, message } => {
            client::send_dialogue(&mut stream, 1, &message).await?;
            let (ack, _) = read_frame(&mut stream).await?;
            if ack.msg_type == MsgType::MsgAck { println!("Delivered."); }
        }
        Cmd::Listen { channel: _ } => {
            println!("Listening... (Ctrl+C to stop)");
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
