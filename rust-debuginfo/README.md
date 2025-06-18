# rust-debuginfo

[![Crates.io](https://img.shields.io/crates/v/rust-debuginfo.svg)](https://crates.io/crates/rust-debuginfo)
[![Documentation](https://docs.rs/rust-debuginfo/badge.svg)](https://docs.rs/rust-debuginfo)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A user-friendly library for interacting with debugging information of Rust compiled artifacts using DWARF.

⚠️ **Experimental Status**: This library is in early development (0.0.x). The API is unstable and subject to breaking changes. We welcome early adopters who are willing to provide feedback!

## Features

- 🚀 **Lazy evaluation** - Parse only what you need, when you need it
- ♻️ **Incremental recomputation** - Powered by [salsa](https://github.com/salsa-rs/salsa) for efficient caching
- 🔍 **Type resolution** - Resolve types from memory addresses
- 📊 **Structured output** - Walk fields and pretty-print complex data structures
- 🦀 **Rust-focused** - Optimized for Rust's specific debug information patterns

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
rust-debuginfo = "0.0.1"
```

## Basic Usage

Here's a simple example of loading a binary and resolving type information from a memory address:

```rust
use rust_debuginfo::{DebugDatabase, TypeInfo};
use anyhow::Result;

fn main() -> Result<()> {
    // Create a new database
    let mut db = DebugDatabase::new()?;
    
    // Load a Rust binary with debug information
    let binary = db.analyze_file("path/to/your/rust/binary")?;
    
    // Resolve type information at a specific address
    let address = 0x12345678;
    if let Some(type_info) = db.resolve_type_at_address(binary, address)? {
        println!("Type at {:#x}: {}", address, type_info.name());
        
        // Walk through fields for structured types
        for field in type_info.fields() {
            println!("  Field '{}': {}", field.name, field.type_name);
        }
    }
    
    Ok(())
}
```

## Memory Address Pretty-Printer Example

For a more complete example showing how to build a memory address pretty-printer, see the [examples/pretty_printer.rs](examples/pretty_printer.rs) file.

## Architecture

This library is designed for use in long-running processes like debuggers:

1. **Efficient caching**: Parse debug information once and reuse it across multiple queries
2. **Lazy parsing**: Only parse the compilation units and DIEs you actually need
3. **Incremental updates**: When binaries change, only recompute affected queries

## Supported Platforms

- **Architectures**: x86_64, aarch64
- **Operating Systems**: macOS, Linux
- **Debug Formats**: DWARF (primary focus)

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

### Development Setup

```bash
# Clone the repository
git clone https://github.com/samscott89/rust_debuginfo
cd rust_debuginfo

# Run tests
cargo test

# Run with examples
cargo run --example pretty_printer
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Built on top of [gimli](https://github.com/gimli-rs/gimli) for DWARF parsing
- Uses [salsa](https://github.com/salsa-rs/salsa) for incremental computation
- Inspired by the needs of Rust debugging tools

## Roadmap

- [ ] Support for more debug formats (PDB on Windows)
- [ ] Enhanced type resolution for complex Rust types
- [ ] Integration with popular debugger frontends
- [ ] Performance optimizations for large binaries

---

**Note**: This is an experimental project. Please report any issues or feature requests on our [GitHub issue tracker](https://github.com/samscott89/rust_debuginfo/issues).