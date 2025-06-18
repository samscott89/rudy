# LLDB Integration for rust-debuginfo

This example demonstrates how to integrate rust-debuginfo with LLDB for enhanced debugging capabilities.

## Features

- **Pretty printing**: Better display of Rust types than LLDB's default
- **Type layout**: Show memory layout of structs with field offsets
- **Type search**: Find types by pattern
- **Cross-referencing**: Navigate between types and their usage

## Setup

1. Build the demo program:
```bash
rustc -g examples/lldb_demo.rs -o target/debug/lldb_demo
```

2. Start the rust-debuginfo server:
```bash
# Option 1: Start server without pre-loading any binary
cargo run --example lldb_server

# Option 2: Start server with a specific binary pre-loaded
cargo run --example lldb_server -- target/debug/lldb_demo

# Option 3: Start server on a custom port
cargo run --example lldb_server -- 9002
```

3. In another terminal, start LLDB with any binary:
```bash
lldb target/debug/lldb_demo
```

4. Load the rust-debuginfo commands:
```
(lldb) command script import examples/rust_debuginfo_lldb.py
```

The server will automatically detect and load the binary you're debugging in LLDB!

## Usage

### Pretty Print Values

```
(lldb) b main
(lldb) run
(lldb) rdi print &session
Type: Session
Source: examples/lldb_demo.rs:38
Value:
  user:
    id: 42
    name: "Alice Smith"
    email: "alice@example.com"
    metadata:
      created: "2024-01-01"
      role: "admin"
  token: "secret-token-12345"
  expires_at: 1234567890
```

### Show Type Layout

```
(lldb) rdi types Session
Type: Session
Size: 120 bytes
Alignment: 8 bytes
Fields:
  [0000] user: User (80 bytes)
  [0050] token: String (24 bytes)
  [0068] expires_at: u64 (8 bytes)
```

### Find Types

```
(lldb) rdi find User
Found 2 types matching 'User':
  User (module: lldb_demo)
  UserMetadata (module: lldb_demo::internal)
```

## Environment Variables

- `RDI_HOST`: Server host (default: 127.0.0.1)
- `RDI_PORT`: Server port (default: 9001)

## Architecture

The integration consists of:

1. **lldb_server.rs**: JSON-RPC server that uses rust-debuginfo to analyze the binary
2. **rust_debuginfo_lldb.py**: LLDB Python script that provides custom commands
3. Communication via TCP sockets using JSON-RPC protocol

## Future Enhancements

- Type summaries for LLDB's variable view
- Synthetic children providers for collections
- Integration with LLDB's expression evaluator
- Caching for better performance