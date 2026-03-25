mod config;
mod types;
mod broker;
mod buffer;
mod relay;
mod server;

use clap::{Parser, Subcommand};

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { port, bind } => server::run(bind, port).await,
        _ => todo!("CLI commands implemented in Task 7"),
    }
}
