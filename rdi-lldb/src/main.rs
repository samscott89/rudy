//! RDI-LLDB Server
//!
//! Event-driven RPC server for LLDB integration with rust-debuginfo

mod protocol;
mod server;
mod session;

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
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rdi_lldb=debug".parse()?),
        )
        .init();

    let args = Args::parse();

    info!("Starting RDI-LLDB server on {}:{}", args.host, args.port);

    // Start the server
    server::run_server(&args.host, args.port)?;

    Ok(())
}
