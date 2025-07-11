# <img src=".github/assets/logo-256.png" alt="Rudy Logo" width="48" align="left" style="margin-right: 10px"> Rudy

<br clear="left"/>

[![Crates.io](https://img.shields.io/crates/v/rudy-db.svg)](https://crates.io/crates/rudy-db)
[![Documentation](https://docs.rs/rudy-db/badge.svg)](https://docs.rs/rudy-db)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A Rust library for interacting with debugging information of compiled artifacts using DWARF format. Provides lazy evaluation and incremental computation for long-running processes like debuggers.

⚠️ **Experimental Status**: This library is in early development (0.0.x). The API is unstable and subject to breaking changes.


> [!IMPORTANT]
> See the [announcement post](https://www.samjs.io/blog/rudy) for more on the rationale/design behind Rudy.

## LLDB Extension

We also provide an example `rudy-lldb` extension that brings the capabilities of `rudy-db` to the `lldb` debugger.

Here's a short demo:

[![rudy-lldb demo](https://asciinema.org/a/CfSY9cLqPwkkB1qxPJrLA302D.svg)](https://asciinema.org/a/CfSY9cLqPwkkB1qxPJrLA302D)

### Installation (rudy-lldb)

For now, the installation process is a little manual:

- Install `rudy-lldb` from source: `cargo install rudy-lldb`.
- Download the Rudy LLDB client: `curl https://raw.githubusercontent.com/samscott89/rudy/refs/heads/main/rudy-lldb/python/rudy_lldb.py -o ~/.lldb/rudy_lldb.py`
- Add Rudy to your `~/.lldbinit` file: `echo "command script import ~/.lldb/rudy_lldb.py" >> ~/.lldbinit`


## Architecture

- **Low-level DWARF parsing** (`rudy-dwarf`) - Parser combinators and visitor patterns abstracting gimli
- **High-level API** (`rudy-db`) - `DebugInfo` wrapper with salsa-based incremental caching  
- **LLDB integration** (`rudy-lldb`) - RPC server for interactive debugging

## Features

- Lazy evaluation using [salsa](https://github.com/salsa-rs/salsa) for incremental computation
- Low-level DWARF parser combinators and visitor structs
- Higher-level `DebugInfo` wrapper for common debugging operations
- Cross-platform support (x86_64, aarch64 on macOS, Linux)


## Basic Usage (rudy-db)

Here's a simple example of loading a binary and resolving type information from a memory address:

```rust
use rudy_db::DebugDb;
use anyhow::Result;

fn main() -> Result<()> {
    // Create a new database
    let mut db = DebugDb::new();

    // Get debug information for a binary
    let debug_info = DebugInfo::new(&db, "/path/to/binary")?;
    
    // Find a function by name
    let function = db.find_function_by_name("my_function")?;

    // get all params:
    for param in &function.params {
        println!("Param: {:?} with type: {}", param.name, param.ty.display_name());
    }
    
    Ok(())
}
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Built on top of [gimli](https://github.com/gimli-rs/gimli) for DWARF parsing
- Uses [salsa](https://github.com/salsa-rs/salsa) for incremental computation

---

**Note**: This is an experimental project. Please report any issues or feature requests on our [GitHub issue tracker](https://github.com/samscott89/rudy/issues).
