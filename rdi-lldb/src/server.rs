//! TCP server implementation for RDI-LLDB

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
};

use anyhow::{Context, Result, anyhow};
use rdi_parser::parse_expression;
use rust_debuginfo::{DebugDb, DebugInfo};
use tracing::{debug, error, info, trace, warn};

use crate::{
    evaluator::EvalContext,
    protocol::{ClientMessage, EventRequest, EventResponseData, ServerMessage},
};

/// Run the RDI-LLDB server
pub fn run_server(host: &str, port: u16) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr)?;
    debug!("Listening on {addr}");

    // create a new debug database instance
    let db = DebugDb::new();

    let mut session_id = 0;

    loop {
        let (stream, addr) = listener.accept()?;
        debug!("New connection from {addr} (session: {session_id})",);

        let db_ref = db.get_sync_ref();
        stream.set_nonblocking(false).unwrap();
        std::thread::spawn(move || {
            let db = db_ref.get_db();
            let mut connection = ClientConnection::new(session_id, stream);
            if let Err(e) = connection.run_server_loop(db) {
                warn!("Connection error: {} (session: {session_id})", e);
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
            debug!("Client disconnected (session: {})", self.session_id);
            return Ok(None);
        }

        let line = self.line_buffer.trim();
        if line.is_empty() {
            return Err(anyhow!("Received empty line"));
        }

        // Parse the client message
        let msg: ClientMessage = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse message: {}", line))?;
        trace!("Received: {msg:#?}");
        Ok(Some(msg))
    }

    fn write_message(&mut self, response: &ServerMessage) -> Result<()> {
        trace!("Sending: {response:#?}");
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
            trace!("Received message: {:?}", msg);
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
        trace!("Received event response: {:?}", response);
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
                let expr = match parse_expression(input) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return Ok(e.into());
                    }
                };

                let mut eval_context = EvalContext::new(debug_info.clone(), self);
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

            "methods" => {
                if args.is_empty() {
                    return Ok(ServerMessage::Error {
                        error: "Usage: methods <type_name_or_expression>".to_string(),
                        backtrace: None,
                    });
                }

                let input = &args[0];
                let result = self.handle_methods_command(input, debug_info);

                match result {
                    Ok(methods) => Ok(ServerMessage::Complete {
                        result: serde_json::to_value(&methods)?,
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

    /// Handle the methods command - discover methods for a type
    fn handle_methods_command(
        &mut self,
        input: &str,
        debug_info: &DebugInfo,
    ) -> Result<MethodDiscoveryResult> {
        // First, try to parse as a type name directly (e.g., "Vec<String>", "HashMap<String, u32>")
        if let Ok(type_def) = debug_info.resolve_type(input) {
            if let Some(type_def) = type_def {
                return Ok(self.discover_methods_for_type(&type_def, debug_info));
            }
        }

        // If not a type name, try to parse as an expression and get its type
        if let Ok(expr) = parse_expression(input) {
            let mut eval_context = EvalContext::new(debug_info.clone(), self);
            if let Ok(type_def) = eval_context.get_expression_type(&expr) {
                return Ok(self.discover_methods_for_type(&type_def, debug_info));
            }
        }

        Err(anyhow!(
            "Could not resolve '{}' as a type or expression",
            input
        ))
    }

    /// Discover methods available for a given type using DWARF debug information
    fn discover_methods_for_type(
        &self,
        type_def: &rust_types::TypeDef,
        debug_info: &DebugInfo,
    ) -> MethodDiscoveryResult {
        // Use the real DWARF-based method discovery
        match debug_info.discover_methods_for_type(type_def) {
            Ok(discovered_methods) => {
                // Convert DiscoveredMethod to MethodInfo
                let methods = discovered_methods
                    .into_iter()
                    .map(|dm| MethodInfo {
                        name: dm.name,
                        signature: dm.signature,
                        description: Some(format!("Function at address {:#x}", dm.address)),
                        callable: dm.callable,
                    })
                    .collect();

                MethodDiscoveryResult {
                    type_name: type_def.display_name(),
                    methods,
                }
            }
            Err(e) => {
                // Fallback to indicating an error occurred
                MethodDiscoveryResult {
                    type_name: type_def.display_name(),
                    methods: vec![MethodInfo {
                        name: "error".to_string(),
                        signature: "discovery failed".to_string(),
                        description: Some(format!("Method discovery failed: {}", e)),
                        callable: false,
                    }],
                }
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MethodDiscoveryResult {
    pub type_name: String,
    pub methods: Vec<MethodInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MethodInfo {
    pub name: String,
    pub signature: String,
    pub description: Option<String>,
    pub callable: bool, // Whether we can actually call this method
}
