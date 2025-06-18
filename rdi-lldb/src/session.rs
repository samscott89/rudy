//! Debug session management

use anyhow::{Result, anyhow};
use rust_debuginfo::DebugInfo;

use crate::protocol::{ClientMessage, ServerMessage};

/// Represents a debugging session with state
pub struct DebugSession<'db> {
    /// Current binary being debugged
    debug_info: DebugInfo<'db>,
    /// Current thread ID
    thread_id: Option<u64>,
    /// Current frame index
    frame_index: Option<usize>,
    /// Message ID counter
    next_msg_id: u64,
}

impl<'db> DebugSession<'db> {
    pub fn new(debug_info: DebugInfo<'db>) -> Self {
        Self {
            debug_info,
            thread_id: None,
            frame_index: None,
            next_msg_id: 1,
        }
    }

    /// Get the next message ID
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id += 1;
        id
    }

    /// Handle a client message
    pub fn handle_message(&mut self, msg: ClientMessage) -> Result<ServerMessage> {
        match msg {
            ClientMessage::Command { id, cmd, args } => self.handle_command(id, &cmd, &args),
            ClientMessage::Init { .. } => {
                Err(anyhow!("Unexpected init message in existing session"))
            }
            ClientMessage::EventResponse { id, data: _ } => {
                Err(anyhow!("Unexpected event response with id {}", id))
            }
        }
    }

    /// Handle a command from the client
    fn handle_command(&mut self, id: u64, cmd: &str, args: &[String]) -> Result<ServerMessage> {
        match cmd {
            "eval" => {
                if args.is_empty() {
                    return Ok(ServerMessage::Error {
                        id,
                        error: "Usage: eval <expression>".to_string(),
                    });
                }

                let _debug_info = &self.debug_info;
                // debug_info.eval(...)

                // TODO: Implement actual expression evaluation
                // This is where we'd parse the expression and potentially
                // return Event messages to read memory, etc.
                Ok(ServerMessage::Error {
                    id,
                    error: "not implemented yet".to_string(),
                })
            }

            "print" => {
                if args.is_empty() {
                    return Ok(ServerMessage::Error {
                        id,
                        error: "Usage: print <expression>".to_string(),
                    });
                }

                // TODO: Implement pretty printing
                Ok(ServerMessage::Error {
                    id,
                    error: "not implemented yet".to_string(),
                })
            }

            _ => Ok(ServerMessage::Error {
                id,
                error: format!("Unknown command: {}", cmd),
            }),
        }
    }
}
