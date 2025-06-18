//! Event-driven RPC protocol for LLDB integration

use serde::{Deserialize, Serialize};

/// Messages sent from client (LLDB) to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Initialize a new session with the binary path
    Init { binary_path: String },
    /// Execute a command
    Command {
        id: u64,
        cmd: String,
        args: Vec<String>,
    },
    /// Response to a server event request
    EventResponse {
        id: u64,
        #[serde(flatten)]
        data: EventResponseData,
    },
}

/// Messages sent from server to client (LLDB)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Request data from LLDB
    Event {
        id: u64,
        #[serde(flatten)]
        event: EventRequest,
    },
    /// Command completed successfully
    Complete { id: u64, result: serde_json::Value },
    /// Command failed
    Error { id: u64, error: String },
}

/// Event types the server can request from LLDB
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum EventRequest {
    /// Read memory at address
    ReadMemory {
        address: u64,
        size: usize,
    },
    /// Read register value
    ReadRegister { name: String },
    /// Get current frame information
    GetFrameInfo,
    /// Get current thread information
    GetThreadInfo,
    /// Evaluate an LLDB expression
    EvaluateLLDBExpression { expr: String },
}

/// Responses to server event requests
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum EventResponseData {
    /// Memory read result
    MemoryData { data: Vec<u8> },
    /// Register read result
    RegisterData {
        value: u64,
    },
    /// Frame information
    FrameInfo {
        pc: u64, // program counter
        sp: u64, // stack pointer
        fp: u64, // frame pointer
    },
    /// Thread information
    ThreadInfo { tid: u64, name: Option<String> },
    /// LLDB expression result
    ExpressionResult { value: String },
    /// Generic error response
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_command_serialization() {
        let msg = ClientMessage::Command {
            id: 1,
            cmd: "eval".to_string(),
            args: vec!["foo.bar".to_string()],
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"Command""#));
        assert!(json.contains(r#""cmd":"eval""#));
    }

    #[test]
    fn test_server_event_serialization() {
        let msg = ServerMessage::Event {
            id: 1,
            event: EventRequest::ReadMemory {
                address: 0x12345678,
                size: 8,
            },
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"Event""#));
        assert!(json.contains(r#""event":"ReadMemory""#));
    }
}
