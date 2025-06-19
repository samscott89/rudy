//! TCP server implementation for RDI-LLDB

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
};

use anyhow::{Context, Result, anyhow};
use rust_debuginfo::{DebugDb, DebugInfo};
use tracing::{debug, error, info, trace};

use crate::{
    evaluator::EvalContext,
    expression,
    protocol::{ClientMessage, EventRequest, EventResponseData, ServerMessage},
};

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
            let mut connection = ClientConnection::new(session_id, stream);
            if let Err(e) = connection.run_server_loop(db) {
                error!("Connection error: {} (session: {session_id})", e);
            }
        });
        session_id += 1;
    }
}

pub struct ClientConnection {
    session_id: usize,
    reader: BufReader<std::net::TcpStream>,
    writer: std::net::TcpStream,
    line_buffer: String,
}

impl ClientConnection {
    pub fn new(session_id: usize, stream: std::net::TcpStream) -> Self {
        let reader = BufReader::new(stream.try_clone().expect("Failed to clone stream"));
        ClientConnection {
            session_id,
            reader,
            writer: stream,
            line_buffer: String::new(),
        }
    }

    pub fn read_next_message(&mut self) -> Result<Option<ClientMessage>> {
        self.line_buffer.clear();
        let bytes_read = self.reader.read_line(&mut self.line_buffer)?;

        if bytes_read == 0 {
            info!("Client disconnected (session: {})", self.session_id);
            return Ok(None);
        }

        let line = self.line_buffer.trim();
        if line.is_empty() {
            return Err(anyhow!("Received empty line"));
        }

        trace!("Received: {}", line);
        // Parse the client message
        let msg: ClientMessage = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse message: {}", line))?;
        debug!("Received: {msg:#?}");
        Ok(Some(msg))
    }

    fn write_message(&mut self, response: &ServerMessage) -> Result<()> {
        debug!("Sending: {response:#?}");
        let response_json = serde_json::to_string(response)? + "\n";
        self.writer.write_all(response_json.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    fn init<'db>(&mut self, db: &'db DebugDb) -> Result<DebugInfo<'db>> {
        // Read the initial message from the client
        // This should be the initialization message
        // containing the path to the binary
        let Some(message) = self.read_next_message()? else {
            // Client disconnected
            return Err(anyhow!("Client disconnected before sending init message"));
        };

        match message {
            ClientMessage::Init { binary_path } => DebugInfo::new(&db, &binary_path)
                .with_context(|| format!("Failed to load binary: {binary_path}")),
            _ => {
                return Err(anyhow!("Invalid message, expected init: {message:?}"));
            }
        }
    }

    /// Handle a single client connection
    pub fn run_server_loop(&mut self, db: DebugDb) -> Result<()> {
        let debug_info = self.init(&db)?;
        loop {
            let msg = match self.read_next_message() {
                Ok(Some(msg)) => msg,
                Ok(None) => {
                    // Client disconnected
                    info!("Client disconnected (session: {})", self.session_id);
                    break;
                }
                Err(e) => {
                    error!("Failed to read message: {}", e);
                    let error_response = ServerMessage::from(e);
                    self.write_message(&error_response)?;

                    continue;
                }
            };
            debug!("Received message: {:?}", msg);
            // Handle the message
            let response = match msg {
                ClientMessage::Command { cmd, args } => {
                    self.handle_command(&cmd, &args, &debug_info)
                }
                ClientMessage::Init { .. } => {
                    anyhow::bail!("Unexpected init message in existing session")
                }
                ClientMessage::EventResponse { .. } => {
                    anyhow::bail!("Unexpected EventResponse message in existing session",)
                }
            }?;

            // Send response
            self.write_message(&response)?;
        }

        Ok(())
    }

    pub fn send_event_request(&mut self, event: EventRequest) -> Result<EventResponseData> {
        let message = ServerMessage::Event { event };
        self.write_message(&message)?;
        // let response = serde_json::to_string(&message)? + "\n";
        // debug!("Sending event: {}", response.trim());
        // self.writer.write_all(response.as_bytes())?;
        // self.writer.flush()?;

        // next, receive the response
        let response = match self.read_next_message()? {
            Some(ClientMessage::EventResponse { data }) => data,
            Some(msg) => {
                return Err(anyhow!("Expected EventResponse, got: {:?}", msg));
            }
            None => {
                return Err(anyhow!(
                    "Client disconnected while waiting for event response"
                ));
            }
        };
        debug!("Received event response: {:?}", response);
        Ok(response)
    }

    /// Handle a command from the client
    fn handle_command(
        &mut self,
        cmd: &str,
        args: &[String],
        debug_info: &DebugInfo,
    ) -> Result<ServerMessage> {
        match cmd {
            "eval" | "print" => {
                if args.is_empty() {
                    return Ok(ServerMessage::Error {
                        error: "Usage: eval <expression>".to_string(),
                        backtrace: None,
                    });
                }

                let input = &args[0];

                // Parse the expression
                let expr = match expression::parse(input) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return Ok(e.into());
                    }
                };

                let eval_context = EvalContext::new(debug_info.clone(), self);
                let result = eval_context
                    .evaluate(&expr)
                    .with_context(|| format!("Failed to evaluate expression: {expr:#?}"));

                match result {
                    Ok(value) => Ok(ServerMessage::Complete {
                        result: serde_json::to_value(&value)?,
                    }),
                    Err(e) => {
                        return Ok(e.into());
                    }
                }
            }

            _ => Ok(ServerMessage::Error {
                error: format!("Unknown command: {cmd}"),
                backtrace: None,
            }),
        }
    }
}
