mod config;
mod types;
mod broker;
mod buffer;
mod relay;
mod server;

use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "synapse-relay", about = "Synapse fleet relay sidecar")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = "7779")]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    Channels,
    Users {
        channel: String,
    },
    Join {
        channel: String,
    },
    Leave {
        channel: String,
    },
    Say {
        channel: String,
        message: Vec<String>,
    },
    Tail {
        channel: String,
        #[arg(long, default_value = "20")]
        count: usize,
    },
    Status,
}

const RELAY_URL: &str = "http://127.0.0.1:7779";

async fn relay_get(path: &str) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}{}", RELAY_URL, path);
    match reqwest::get(&url).await {
        Ok(response) => {
            if response.status().is_success() {
                Ok(response.json().await?)
            } else {
                let error_text = response.text().await.unwrap_or_default();
                if let Ok(error_obj) = serde_json::from_str::<serde_json::Value>(&error_text) {
                    if let Some(error_msg) = error_obj.get("error").and_then(|e| e.as_str()) {
                        anyhow::bail!("{}", error_msg);
                    }
                }
                anyhow::bail!("{}", error_text);
            }
        }
        Err(e) => {
            if e.is_connect() {
                eprintln!("synapse-relay is not running. Start with: synapse-relay serve");
                std::process::exit(1);
            }
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

async fn relay_post(path: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}{}", RELAY_URL, path);
    let client = reqwest::Client::new();
    match client.post(&url).json(&body).send().await {
        Ok(response) => {
            if response.status().is_success() {
                Ok(response.json().await?)
            } else {
                let error_text = response.text().await.unwrap_or_default();
                if let Ok(error_obj) = serde_json::from_str::<serde_json::Value>(&error_text) {
                    if let Some(error_msg) = error_obj.get("error").and_then(|e| e.as_str()) {
                        anyhow::bail!("{}", error_msg);
                    }
                }
                anyhow::bail!("{}", error_text);
            }
        }
        Err(e) => {
            if e.is_connect() {
                eprintln!("synapse-relay is not running. Start with: synapse-relay serve");
                std::process::exit(1);
            }
            Err(anyhow::anyhow!("{}", e))
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { port, bind } => server::run(bind, port).await,
        Commands::Channels => {
            let response = relay_get("/channels").await?;
            if let Some(channels) = response.get("channels").and_then(|c| c.as_array()) {
                for channel in channels {
                    if let Some(name) = channel.as_str() {
                        println!("{}", name);
                    }
                }
            }
            Ok(())
        }
        Commands::Users { channel } => {
            let encoded_channel = urlencoding::encode(&channel);
            let response = relay_get(&format!("/users?channel={}", encoded_channel)).await?;
            if let Some(users) = response.get("users").and_then(|u| u.as_array()) {
                for user in users {
                    if let Some(name) = user.as_str() {
                        println!("{}", name);
                    }
                }
            }
            Ok(())
        }
        Commands::Join { channel } => {
            relay_post("/subscribe", json!({ "channel": channel })).await?;
            println!("Joined {}", channel);
            Ok(())
        }
        Commands::Leave { channel } => {
            relay_post("/leave", json!({ "channel": channel })).await?;
            println!("Left {}", channel);
            Ok(())
        }
        Commands::Say { channel, message } => {
            let text = message.join(" ");
            relay_post("/send", json!({ "channel": channel, "text": text })).await?;
            Ok(())
        }
        Commands::Tail { channel, count } => {
            let encoded_channel = urlencoding::encode(&channel);
            let response = relay_get(&format!("/poll?channel={}&since=0", encoded_channel)).await?;
            if let Some(messages) = response.get("messages").and_then(|m| m.as_array()) {
                let start = if messages.len() > count {
                    messages.len() - count
                } else {
                    0
                };
                for msg in &messages[start..] {
                    if let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) {
                        if msg_type == "dialogue" {
                            if let Some(text) = msg.get("text").and_then(|t| t.as_str()) {
                                println!("{}", text);
                            }
                        } else if msg_type == "work" {
                            if let Some(payload) = msg.get("payload") {
                                println!("{}", serde_json::to_string_pretty(payload)?);
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        Commands::Status => {
            let response = relay_get("/status").await?;
            if let Some(channels) = response.get("channels").and_then(|c| c.as_array()) {
                println!("{:<20}{:<12}{:<10}", "CHANNEL", "CONNECTED", "BUFFERED");
                for channel in channels {
                    if let (Some(name), Some(connected), Some(buffered)) = (
                        channel.get("channel").and_then(|c| c.as_str()),
                        channel.get("connected").and_then(|c| c.as_bool()),
                        channel.get("buffered").and_then(|b| b.as_u64()),
                    ) {
                        let connected_str = if connected { "yes" } else { "no" };
                        println!("{:<20}{:<12}{:<10}", name, connected_str, buffered);
                    }
                }
            }
            Ok(())
        }
    }
}
