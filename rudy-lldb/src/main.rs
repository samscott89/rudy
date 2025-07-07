//! rudy-lldb Server
//!
//! Event-driven RPC server for LLDB integration with rudy-db

mod evaluator;
mod protocol;
mod server;

use anyhow::Result;
use clap::Parser;
use tracing::info;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "9001")]
    port: u16,

    /// Host to bind to
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("salsa=warn".parse()?),
        )
        .init();

    let args = Args::parse();

    info!("Starting rudy-lldb server on {}:{}", args.host, args.port);

    // Start the server
    server::run_server(&args.host, args.port)?;

    Ok(())
}
