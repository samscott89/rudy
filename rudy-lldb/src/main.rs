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
    /// Install the LLDB client script
    Install {
        /// Directory to install the script to
        #[arg(short, long, default_value = "~/.lldb")]
        dir: String,

        /// Skip adding to ~/.lldbinit
        #[arg(short, long)]
        skip_lldbinit: bool,

        /// Automatically answer yes to all prompts
        #[arg(short = 'y', long = "yes")]
        assume_yes: bool,
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
        Command::Install {
            dir,
            skip_lldbinit,
            assume_yes,
        } => {
            install_lldb_client(&dir, skip_lldbinit, assume_yes)?;
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

fn install_lldb_client(install_dir: &str, skip_lldbinit: bool, assume_yes: bool) -> Result<()> {
    use std::fs;
    use std::io::{self, BufRead, Write};
    use std::path::PathBuf;

    // Expand tilde in path
    let install_dir = if install_dir.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        install_dir.replacen("~/", &format!("{home}/"), 1)
    } else {
        install_dir.to_string()
    };

    // Create install directory if it doesn't exist
    fs::create_dir_all(&install_dir)?;

    let script_path = PathBuf::from(&install_dir).join("rudy_lldb.py");

    // Check if script already exists
    if script_path.exists() && !assume_yes {
        print!(
            "Script already exists at {}. Overwrite? [y/N]: ",
            script_path.display()
        );
        io::stdout().flush()?;

        let stdin = io::stdin();
        let mut response = String::new();
        stdin.lock().read_line(&mut response)?;

        if !response.trim().to_lowercase().starts_with('y') {
            println!("Installation cancelled.");
            return Ok(());
        }
    }

    println!("Fetching latest rudy-lldb release from GitHub...");

    // Get all releases and find the latest rudy-lldb release
    let releases_url = "https://api.github.com/repos/samscott89/rudy/releases";
    let releases: Vec<serde_json::Value> = ureq::get(releases_url)
        .header("User-Agent", "rudy-lldb-installer")
        .call()
        .map_err(|e| anyhow::anyhow!("Failed to fetch releases: {}", e))?
        .body_mut()
        .read_json()
        .map_err(|e| anyhow::anyhow!("Failed to parse releases JSON: {}", e))?;

    // Find the latest rudy-lldb release (tag starts with "rudy-lldb-")
    let release_json = releases
        .into_iter()
        .find(|release| {
            release["tag_name"]
                .as_str()
                .map(|tag| tag.starts_with("rudy-lldb-"))
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("No rudy-lldb releases found"))?;

    // Find the rudy_lldb.py asset
    let assets = release_json["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No assets found in release"))?;

    let script_asset = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some("rudy_lldb.py"))
        .ok_or_else(|| anyhow::anyhow!("rudy_lldb.py not found in release assets"))?;

    let download_url = script_asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid download URL"))?;

    println!(
        "Downloading rudy_lldb.py from release {}...",
        release_json["tag_name"].as_str().unwrap_or("unknown")
    );

    // Download the script
    let content = ureq::get(download_url)
        .header("User-Agent", "rudy-lldb-installer")
        .call()
        .map_err(|e| anyhow::anyhow!("Failed to download script: {}", e))?
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

    // Write the script
    fs::write(&script_path, content)?;
    println!("✓ Downloaded script to {}", script_path.display());

    // Handle ~/.lldbinit
    if !skip_lldbinit {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let lldbinit_path = PathBuf::from(&home).join(".lldbinit");
        let import_line = format!("command script import {}", script_path.display());

        // Check if import line already exists
        let mut already_added = false;
        if lldbinit_path.exists() {
            let content = fs::read_to_string(&lldbinit_path)?;
            if content.contains(&import_line) {
                already_added = true;
            }
        }

        if already_added {
            println!("✓ Import already exists in ~/.lldbinit");
        } else {
            let should_add = assume_yes || {
                print!("Add import to ~/.lldbinit? [Y/n]: ");
                io::stdout().flush()?;

                let stdin = io::stdin();
                let mut response = String::new();
                stdin.lock().read_line(&mut response)?;

                let response = response.trim().to_lowercase();
                response.is_empty() || response.starts_with('y')
            };

            if should_add {
                // Append to .lldbinit
                use std::fs::OpenOptions;
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&lldbinit_path)?;

                writeln!(file, "{import_line}")?;
                println!("✓ Added import to ~/.lldbinit");
            } else {
                println!("\nTo manually add the import, add this line to your ~/.lldbinit:");
                println!("  {import_line}");
            }
        }
    } else {
        println!("\nTo manually add the import, add this line to your ~/.lldbinit:");
        println!("  command script import {}", script_path.display());
    }

    println!("\n✓ Installation complete!");
    println!("You can now use the 'rd' command in LLDB for enhanced Rust debugging.");

    Ok(())
}
