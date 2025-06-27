# rudy-lldb Demo

This demo shows how to use the rudy-lldb extension for enhanced Rust debugging.

## Setup

1. **Install the extension:**

   TODO: make automatic

   - Add `command script import <path to repo>/python/rudy_lldb.py` to `~/.lldbinit`


2. **Build the demo program:**
   ```bash
   rustc -g ../examples/lldb_demo.rs -o target/debug/lldb_demo -C split-debuginfo=unpacked
   ```

3. **Start LLDB:**
   ```bash
   lldb target/debug/lldb_demo
   ```

## Demo Script

```lldb
# Set a breakpoint at main
(lldb) b main
Breakpoint 1: where = lldb_demo`lldb_demo::main + 28 at lldb_demo.rs:39, address = 0x000000010000151c

# Run the program
(lldb) run
Process 12345 launched: 'target/debug/lldb_demo' (arm64)
Process 12345 stopped
* thread #1, queue = 'com.apple.main-thread', stop reason = breakpoint 1.1
    frame #0: 0x000000010000151c lldb_demo`lldb_demo::main + 28 at lldb_demo.rs:39
   36   
   37   fn main() {
   38       let user = User::new(42, "Alice Smith", "alice@example.com");
-> 39       let session = create_session(user);
   40       
   41       // Set a breakpoint here to inspect with LLDB
   42       println!("Session created: {:?}", session);

# Check rudy-lldb status
(lldb) rd status
✓ Rudy server is already running

# Try to evaluate a Rust expression (will fail with "not implemented" for now)
(lldb) rd eval user.id
Error: not implemented yet

# Try pretty printing
(lldb) rd print user
Error: not implemented yet

# Step to the next line to get the session object
(lldb) n
Process 12345 stopped
* thread #1, queue = 'com.apple.main-thread', stop reason = step over
    frame #0: 0x0000000100001540 lldb_demo`lldb_demo::main + 64 at lldb_demo.rs:42
   39       let session = create_session(user);
   40       
   41       // Set a breakpoint here to inspect with LLDB
-> 42       println!("Session created: {:?}", session);

# Try with the session object
(lldb) rd eval session.user.name
Error: not implemented yet
```

## Current Status

The infrastructure is working:
- ✅ Rudy server starts automatically
- ✅ LLDB extension loads and connects
- ✅ Event-driven protocol handles basic commands
- ✅ Server can receive binary path and initialize sessions
- ❌ Expression evaluation not yet implemented
- ❌ Pretty printing not yet implemented

## Next Steps

1. **Implement expression parser** in the Rust server
2. **Add memory reading capabilities** for evaluating expressions
3. **Create pretty printers** for common Rust types
4. **Add method introspection**


## Architecture

```
LLDB → Python Extension → TCP Socket → Rudy Server → rudy-lldb
  ↑                                           ↓
  ←─── Events (memory reads, etc.) ←─────────────
```

The event-driven protocol allows the server to request data from LLDB
as needed during expression evaluation.