//! TCP server implementation for RDI-LLDB

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
};

use anyhow::{Context, Result, anyhow};
use rust_debuginfo::{DebugDb, DebugInfo};
use tracing::{debug, error, info, warn};

use crate::protocol::{ClientMessage, ServerMessage};

/// Run the RDI-LLDB server
pub fn run_server(host: &str, port: u16) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr)?;
    info!("Listening on {addr}");

    // create a new debug database instance
    let db = DebugDb::new();

    let mut session_id = 0;

    loop {
        let (stream, addr) = listener.accept()?;
        info!("New connection from {addr} (session: {session_id})",);

        let db_ref = db.get_sync_ref();
        stream.set_nonblocking(false).unwrap();
        std::thread::spawn(move || {
            let db = db_ref.get_db();
            if let Err(e) = handle_connection(session_id, stream, db) {
                error!("Connection error: {} (session: {session_id})", e);
            }
        });
        session_id += 1;
    }
}

fn read_next_message(
    session_id: usize,
    reader: &mut BufReader<std::net::TcpStream>,
) -> Result<Option<ClientMessage>> {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;

    if bytes_read == 0 {
        info!("Client disconnected (session: {session_id})");
        return Ok(None);
    }

    let line = line.trim();
    if line.is_empty() {
        return Err(anyhow!("Received empty line"));
    }

    debug!("Received: {}", line);
    // Parse the client message
    let msg: ClientMessage =
        serde_json::from_str(line).with_context(|| format!("Failed to parse message: {}", line))?;
    Ok(Some(msg))
}

/// Handle a single client connection
fn handle_connection(session_id: usize, stream: std::net::TcpStream, db: DebugDb) -> Result<()> {
    // let (reader, mut writer) = stream.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    // Read the initial message from the client
    // This should be the initialization message
    // containing the path to the binary
    let Some(message) = read_next_message(session_id, &mut reader)? else {
        // Client disconnected
        return Ok(());
    };

    let mut session = match message {
        ClientMessage::Init { binary_path } => {
            let debug_info = DebugInfo::new(&db, &binary_path)
                .with_context(|| format!("Failed to load binary: {binary_path}"))?;

            let session = crate::session::DebugSession::new(debug_info);
            info!("Starting new session (id: {session_id})");
            session
        }
        _ => {
            return Err(anyhow!("Invalid message: {message:?}"));
        }
    };

    loop {
        let msg = match read_next_message(session_id, &mut reader) {
            Ok(Some(msg)) => msg,
            Ok(None) => {
                // Client disconnected
                info!("Client disconnected (session: {session_id})");
                break;
            }
            Err(e) => {
                error!("Failed to read message: {}", e);
                let error_response = ServerMessage::Error {
                    id: 0,
                    error: format!("Failed to read message: {}", e),
                };
                let response = serde_json::to_string(&error_response)? + "\n";
                writer.write_all(response.as_bytes())?;
                writer.flush()?;

                continue;
            }
        };
        debug!("Received message: {:?}", msg);
        // Handle the message
        let response = match session.handle_message(msg) {
            Ok(response) => response,
            Err(e) => ServerMessage::Error {
                id: 0,
                error: format!("Internal error: {}", e),
            },
        };

        // Send response
        let response_json = serde_json::to_string(&response)? + "\n";
        debug!("Sending: {}", response_json.trim());
        writer.write_all(response_json.as_bytes())?;
        writer.flush()?;
    }

    Ok(())
}
