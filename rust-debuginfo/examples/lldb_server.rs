//! LLDB integration server for rust-debuginfo
//!
//! This server provides a JSON-RPC interface that LLDB can call to get
//! enhanced debug information and pretty printing.

use rust_debuginfo::{DebugDb, DebugInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    method: String,
    params: serde_json::Value,
    id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    result: Option<serde_json::Value>,
    error: Option<String>,
    id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrettyPrintParams {
    address: String, // hex string like "0x12345678"
    max_depth: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TypeLayoutParams {
    type_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FindTypeParams {
    pattern: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoadBinaryParams {
    binary_path: String,
}

struct ServerState<'db> {
    db: &'db DebugDb,
    binaries: HashMap<String, DebugInfo<'db>>,
    current_binary: Option<String>,
}

fn handle_request(state: &mut ServerState, request: Request) -> Response {
    match request.method.as_str() {
        "load_binary" => {
            let params: LoadBinaryParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response {
                        result: None,
                        error: Some(format!("Invalid params: {}", e)),
                        id: request.id,
                    }
                }
            };

            // Load the binary if not already loaded
            if !state.binaries.contains_key(&params.binary_path) {
                match DebugInfo::new(state.db, &params.binary_path) {
                    Ok(info) => {
                        state.binaries.insert(params.binary_path.clone(), info);
                        state.current_binary = Some(params.binary_path.clone());
                        Response {
                            result: Some(serde_json::json!({
                                "status": "loaded",
                                "binary": params.binary_path
                            })),
                            error: None,
                            id: request.id,
                        }
                    }
                    Err(e) => Response {
                        result: None,
                        error: Some(format!("Failed to load binary: {}", e)),
                        id: request.id,
                    }
                }
            } else {
                state.current_binary = Some(params.binary_path.clone());
                Response {
                    result: Some(serde_json::json!({
                        "status": "already_loaded",
                        "binary": params.binary_path
                    })),
                    error: None,
                    id: request.id,
                }
            }
        }
        "pretty_print" => {
            let params: PrettyPrintParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response {
                        result: None,
                        error: Some(format!("Invalid params: {}", e)),
                        id: request.id,
                    };
                }
            };

            // Parse hex address
            let address = match params.address.strip_prefix("0x") {
                Some(hex) => match u64::from_str_radix(hex, 16) {
                    Ok(addr) => addr,
                    Err(e) => {
                        return Response {
                            result: None,
                            error: Some(format!("Invalid address: {}", e)),
                            id: request.id,
                        };
                    }
                },
                None => {
                    return Response {
                        result: None,
                        error: Some("Address must start with 0x".to_string()),
                        id: request.id,
                    };
                }
            };

            // TODO: Use rust-debuginfo to resolve type at address and pretty print
            // For now, return a mock response
            let result = serde_json::json!({
                "type": "MyStruct",
                "value": {
                    "field1": 42,
                    "field2": "hello",
                },
                "source": "src/main.rs:15"
            });

            Response {
                result: Some(result),
                error: None,
                id: request.id,
            }
        }
        "type_layout" => {
            let params: TypeLayoutParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response {
                        result: None,
                        error: Some(format!("Invalid params: {}", e)),
                        id: request.id,
                    };
                }
            };

            // TODO: Use rust-debuginfo to get type layout
            let result = serde_json::json!({
                "name": params.type_name,
                "size": 32,
                "align": 8,
                "fields": [
                    {"name": "id", "offset": 0, "size": 8, "type": "u64"},
                    {"name": "data", "offset": 8, "size": 24, "type": "String"}
                ]
            });

            Response {
                result: Some(result),
                error: None,
                id: request.id,
            }
        }
        "find_type" => {
            let params: FindTypeParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => {
                    return Response {
                        result: None,
                        error: Some(format!("Invalid params: {}", e)),
                        id: request.id,
                    };
                }
            };

            // TODO: Search for types matching pattern
            let result = serde_json::json!({
                "types": [
                    {"name": "MyStruct", "module": "my_app"},
                    {"name": "MyEnum", "module": "my_app::types"}
                ]
            });

            Response {
                result: Some(result),
                error: None,
                id: request.id,
            }
        }
        _ => Response {
            result: None,
            error: Some(format!("Unknown method: {}", request.method)),
            id: request.id,
        },
    }
}

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<ServerState>>) {
    let reader = BufReader::new(stream.try_clone().unwrap());

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading line: {}", e);
                break;
            }
        };

        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error parsing request: {}", e);
                continue;
            }
        };

        let response = {
            let mut state = state.lock().unwrap();
            handle_request(&mut state, request)
        };
        let response_str = serde_json::to_string(&response).unwrap();

        if let Err(e) = writeln!(stream, "{}", response_str) {
            eprintln!("Error writing response: {}", e);
            break;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port = args
        .get(1)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(9001);

    // Initialize server state
    let db = Box::leak(Box::new(DebugDb::new()));
    let mut state = ServerState {
        db,
        binaries: HashMap::new(),
        current_binary: None,
    };

    // If a binary path was provided, pre-load it
    if args.len() > 1 && !args[1].parse::<u16>().is_ok() {
        let binary_path = &args[1];
        match DebugInfo::new(db, binary_path) {
            Ok(info) => {
                println!("Pre-loaded binary: {}", binary_path);
                state.binaries.insert(binary_path.clone(), info);
                state.current_binary = Some(binary_path.clone());
            }
            Err(e) => {
                eprintln!("Warning: Failed to pre-load binary: {}", e);
            }
        }
    }

    let state = Arc::new(Mutex::new(state));

    println!("Starting LLDB server on port {}", port);
    println!("Usage: The server will automatically load binaries as LLDB requests them");

    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                println!("Client connected: {}", stream.peer_addr().unwrap());
                handle_client(stream, state.clone());
            }
            Err(e) => {
                eprintln!("Connection error: {}", e);
            }
        }
    }
}
