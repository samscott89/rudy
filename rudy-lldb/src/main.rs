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
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Start the rudy-lldb server
    Start {
        /// Port to listen on
        #[arg(short, long, default_value = "5737")]
        port: u16,

        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Stop the running rudy-lldb server
    Stop {
        /// Port of the server to stop
        #[arg(short, long, default_value = "5737")]
        port: u16,

        /// Host of the server to stop
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Command::Start { port, host } => {
            // Initialize tracing
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("salsa=warn".parse()?),
                )
                .init();

            info!("Starting rudy-lldb server on {}:{}", host, port);
            server::run_server(&host, port)?;
        }
        Command::Stop { port, host } => {
            stop_server(&host, port)?;
        }
    }

    Ok(())
}

fn stop_server(host: &str, port: u16) -> Result<()> {
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::Duration;

    println!("Attempting to stop rudy-lldb server at {host}:{port}");

    // Try to connect to the server
    match TcpStream::connect_timeout(&format!("{host}:{port}").parse()?, Duration::from_secs(5)) {
        Ok(mut stream) => {
            // Send shutdown message
            let shutdown_msg =
                r#"{"type":"Command","cmd":"shutdown","args":[]}"#.to_string() + "\n";
            stream.write_all(shutdown_msg.as_bytes())?;
            stream.flush()?;
            println!("Shutdown command sent successfully");
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to connect to server at {host}:{port} - {e}");
            eprintln!("The server may not be running");
            Err(e.into())
        }
    }
}
