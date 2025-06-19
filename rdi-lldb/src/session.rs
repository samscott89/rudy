//! Debug session management

use anyhow::{Result, anyhow};
use rust_debuginfo::DebugInfo;
use std::collections::HashMap;

use crate::evaluator::{self, EvalContext, RemoteDataAccess};
use crate::expression::{self, Expression};
use crate::protocol::{ClientMessage, EventRequest, EventResponseData, ServerMessage};

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
            ClientMessage::EventResponse { id, data } => self.handle_event_response(id, data),
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

                let input = &args[0];

                // Parse the expression
                let expr = match expression::parse(input) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return Ok(ServerMessage::Error {
                            id,
                            error: format!("Parse error: {}", e),
                        });
                    }
                };

                // Store the expression for later evaluation
                // We'll handle the actual evaluation in handle_event_response
                self.pending_evaluation = Some((id, expr, None));

                // Request frame info to start evaluation
                Ok(ServerMessage::Event {
                    id,
                    event: EventRequest::GetFrameInfo,
                })
            }

            "print" => {
                // Redirect to eval since they're now the same
                self.handle_command(id, "eval", args)
            }

            _ => Ok(ServerMessage::Error {
                id,
                error: format!("Unknown command: {}", cmd),
            }),
        }
    }

    /// Handle an event response from the client
    fn handle_event_response(&mut self, id: u64, data: EventResponseData) -> Result<ServerMessage> {
        match &mut self.pending_evaluation {
            Some((eval_id, expr, eval_state)) if *eval_id == id => {
                let expr = expr.clone();
                let eval_state = eval_state.take();

                match eval_state {
                    None => {
                        // This is the initial GetFrameInfo response
                        match data {
                            EventResponseData::FrameInfo { pc, sp, fp } => {
                                // Create evaluation context
                                let context = EvalContext {
                                    debug_info: &self.debug_info,
                                    pc,
                                    sp,
                                    fp,
                                    memory_cache: HashMap::new(),
                                    register_cache: HashMap::new(),
                                };

                                // Start evaluation
                                match evaluator::evaluate(&expr, &context, id)? {
                                    EvaluationState::Complete(result) => {
                                        // Evaluation complete, clear pending state
                                        self.pending_evaluation = None;
                                        Ok(ServerMessage::Complete {
                                            id,
                                            result: serde_json::json!({
                                                "value": result.value,
                                                "type": result.type_name,
                                                "pretty": result.pretty
                                            }),
                                        })
                                    }
                                    EvaluationState::NeedEvent {
                                        event,
                                        continuation: _,
                                    } => {
                                        // Need more data, store the state and send the event
                                        // For now, we'll simplify and not handle complex continuations
                                        self.pending_evaluation = None;
                                        Ok(ServerMessage::Error {
                                            id,
                                            error: "Complex evaluation not yet implemented"
                                                .to_string(),
                                        })
                                    }
                                }
                            }
                            _ => {
                                self.pending_evaluation = None;
                                Ok(ServerMessage::Error {
                                    id,
                                    error: "Expected FrameInfo response".to_string(),
                                })
                            }
                        }
                    }
                    Some(_state) => {
                        // Handle continuation from evaluation state
                        // For now, just return an error
                        self.pending_evaluation = None;
                        Ok(ServerMessage::Error {
                            id,
                            error: "Complex evaluation continuations not yet implemented"
                                .to_string(),
                        })
                    }
                }
            }
            _ => Ok(ServerMessage::Error {
                id,
                error: format!("No pending evaluation for id {}", id),
            }),
        }
    }
}
