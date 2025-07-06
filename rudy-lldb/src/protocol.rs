//! Event-driven RPC protocol for LLDB integration

use std::fmt;

use serde::{Deserialize, Serialize};

/// Argument for method execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodArgument {
    /// The argument value (serialized as hex string for addresses, or as string for primitives)
    pub value: String,
    /// The type of the argument for proper casting
    pub arg_type: ArgumentType,
}

/// Supported argument types for method execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArgumentType {
    /// Pointer/reference argument (e.g., &self, &mut self, *const T)
    Pointer,
    /// Integer argument (i32, u64, usize, etc.)
    Integer,
    /// Boolean argument
    Bool,
    /// Floating point argument
    Float,
}

/// Result of method execution - handles both simple and complex return types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MethodCallResult {
    /// Simple value returned in registers (primitives)
    SimpleValue { value: u64, return_type: String },
    /// Complex value returned via pointer (String, Vec, structs, etc.)
    ComplexPointer {
        address: u64,
        size: usize,
        return_type: String,
    },
}

/// Messages sent from client (LLDB) to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Initialize a new session with the binary path
    Init { binary_path: String },
    /// Execute a command
    Command { cmd: String, args: Vec<String> },
    /// Response to a server event request
    EventResponse {
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
        #[serde(flatten)]
        event: EventRequest,
    },
    /// Command completed successfully
    Complete { result: serde_json::Value },
    /// Command failed
    Error {
        error: String,
        backtrace: Option<String>,
    },
}

impl From<anyhow::Error> for ServerMessage {
    fn from(err: anyhow::Error) -> Self {
        ServerMessage::Error {
            error: format!("{err:?}"),
            backtrace: None, //Some(err.backtrace().to_string()),
        }
    }
}

/// Event types the server can request from LLDB
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum EventRequest {
    /// Read memory at address
    ReadMemory { address: u64, size: usize },
    /// Read register value by name
    ReadRegister { name: String },
    /// Read register value by index
    ReadRegisterByIndex { index: usize },
    /// Get current frame information
    GetFrameInfo,
    /// Get current thread information
    GetThreadInfo,
    /// Get the base load address of the binary
    GetBaseAddress,
    /// Evaluate an LLDB expression
    EvaluateLLDBExpression { expr: String },
    /// Execute a method by calling it directly
    ExecuteMethod {
        method_address: u64,
        base_address: u64,
        args: Vec<MethodArgument>,
        /// Size in bytes for complex return types that use return-via-pointer ABI.
        /// None for simple types returned in registers, Some(size) for complex types.
        return_type_size: Option<usize>,
    },
    /// Execute a function by calling it directly
    ExecuteFunction {
        function_address: u64,
        args: Vec<MethodArgument>,
        /// Size in bytes for complex return types that use return-via-pointer ABI.
        /// None for simple types returned in registers, Some(size) for complex types.
        return_type_size: Option<usize>,
    },
}

impl fmt::Debug for EventRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadMemory { address, size } => f
                .debug_struct("ReadMemory")
                .field("address", &format!("{address:#x}"))
                .field("size", size)
                .finish(),
            Self::ReadRegister { name } => {
                f.debug_struct("ReadRegister").field("name", name).finish()
            }
            Self::ReadRegisterByIndex { index } => f
                .debug_struct("ReadRegisterByIndex")
                .field("index", index)
                .finish(),
            Self::GetFrameInfo => write!(f, "GetFrameInfo"),
            Self::GetThreadInfo => write!(f, "GetThreadInfo"),
            Self::GetBaseAddress => write!(f, "GetBaseAddress"),
            Self::EvaluateLLDBExpression { expr } => f
                .debug_struct("EvaluateLLDBExpression")
                .field("expr", expr)
                .finish(),
            Self::ExecuteMethod {
                method_address,
                base_address,
                args,
                return_type_size,
            } => f
                .debug_struct("ExecuteMethod")
                .field("method_address", &format!("{method_address:#x}"))
                .field("base_address", &format!("{base_address:#x}"))
                .field("args", args)
                .field("return_type_size", return_type_size)
                .finish(),
            Self::ExecuteFunction {
                function_address,
                args,
                return_type_size,
            } => f
                .debug_struct("ExecuteFunction")
                .field("function_address", &format!("{function_address:#x}"))
                .field("args", args)
                .field("return_type_size", return_type_size)
                .finish(),
        }
    }
}

/// Responses to server event requests
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum EventResponseData {
    /// Memory read result
    MemoryData { data: Vec<u8> },
    /// Register read result
    RegisterData { value: u64 },
    /// Frame information
    FrameInfo {
        pc: u64, // program counter
        sp: u64, // stack pointer
        fp: u64, // frame pointer
    },
    /// Thread information
    ThreadInfo { tid: u64, name: Option<String> },
    /// Base address information
    BaseAddress { address: u64 },
    /// LLDB expression result
    ExpressionResult { value: String },
    /// Method execution result
    MethodResult { result: MethodCallResult },
    /// Function execution result
    FunctionResult { result: MethodCallResult },
    /// Generic error response
    Error { message: String },
}

impl fmt::Debug for EventResponseData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryData { data } => f.debug_struct("MemoryData").field("data", data).finish(),
            Self::RegisterData { value } => f
                .debug_struct("RegisterData")
                .field("value", &format!("{value:#x}"))
                .finish(),
            Self::FrameInfo { pc, sp, fp } => f
                .debug_struct("FrameInfo")
                .field("pc", &format!("{pc:#x}"))
                .field("sp", &format!("{sp:#x}"))
                .field("fp", &format!("{fp:#x}"))
                .finish(),
            Self::ThreadInfo { tid, name } => f
                .debug_struct("ThreadInfo")
                .field("tid", tid)
                .field("name", name)
                .finish(),
            Self::BaseAddress { address } => f
                .debug_struct("BaseAddress")
                .field("address", &format!("{address:#x}"))
                .finish(),
            Self::ExpressionResult { value } => f
                .debug_struct("ExpressionResult")
                .field("value", value)
                .finish(),
            Self::MethodResult { result } => f
                .debug_struct("MethodResult")
                .field("result", result)
                .finish(),
            Self::FunctionResult { result } => f
                .debug_struct("FunctionResult")
                .field("result", result)
                .finish(),
            Self::Error { message } => f.debug_struct("Error").field("message", message).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_command_serialization() {
        let msg = ClientMessage::Command {
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
