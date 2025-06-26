//! Memory address pretty-printer example
//!
//! This example demonstrates how to use rust-debuginfo to resolve memory addresses
//! to their types and pretty-print structured data.

use anyhow::Result;
use rust_debuginfo::{DataResolver, DebugDb, DebugInfo, Value};
use std::env;

/// Example data resolver that reads from a memory dump or snapshot
struct MemorySnapshot {
    base: u64,
    // In a real implementation, this would contain the actual memory data
}

impl DataResolver for MemorySnapshot {
    fn base_address(&self) -> u64 {
        self.base
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        // In a real implementation, this would read from the memory snapshot
        // For this example, we'll return dummy data
        Ok(vec![0; size])
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        // Return empty registers for this example
        Ok(vec![])
    }
}

fn print_value(value: &Value, indent: usize) {
    let indent_str = " ".repeat(indent);

    match value {
        Value::Scalar { ty, value } => {
            println!("{indent_str}{ty}: {value}");
        }
        Value::Array { ty, items } => {
            println!("{indent_str}[{ty}; {}]", items.len());
            for (i, item) in items.iter().enumerate() {
                print!("{indent_str}  [{i}] = ");
                print_value(item, 0);
            }
        }
        Value::Struct { ty, fields } => {
            println!("{indent_str}{ty} {{");
            for (name, value) in fields {
                print!("{indent_str}  {name}: ");
                print_value(value, indent + 4);
            }
            println!("{indent_str}}}");
        }
        Value::Tuple { ty, entries } => {
            println!("{indent_str}({ty})");
            for (i, entry) in entries.iter().enumerate() {
                print!("{indent_str}  [{i}] = ");
                print_value(entry, indent + 4);
            }
        }
        Value::Map { ty, entries } => {
            println!("{indent_str}{ty} {{");
            for (key, value) in entries {
                print!("{indent_str}  ");
                print_value(key, indent);
                print!(": ");
                print_value(value, indent + 4);
            }
            println!("{indent_str}}}");
        }
    }
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Get command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <binary_path> <address>", args[0]);
        eprintln!("Example: {} ./target/debug/myapp 0x12345678", args[0]);
        std::process::exit(1);
    }

    let binary_path = &args[1];
    let address =
        u64::from_str_radix(args[2].trim_start_matches("0x"), 16).expect("Invalid address format");

    // Create debug database and load binary
    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, binary_path)?;

    // Resolve the address to a source location
    if let Some(location) = debug_info.address_to_line(address) {
        println!("Address {:#x} is at:", address);
        println!("  Function: {}", location.function);
        println!("  File: {}", location.file);
        println!("  Line: {}", location.line);
        println!();
    }

    // Create a data resolver (in a real debugger, this would read from the process)
    let resolver = MemorySnapshot { base: 0x100000 };

    // Resolve variables at this address
    let (params, locals, globals) = debug_info.resolve_variables_at_address(address, &resolver)?;

    if !params.is_empty() {
        println!("Parameters:");
        for var in &params {
            println!(
                "  {} ({})",
                var.name,
                var.ty.as_ref().map_or("?", |t| &t.name)
            );
            if let Some(value) = &var.value {
                print_value(value, 4);
            }
        }
        println!();
    }

    if !locals.is_empty() {
        println!("Local variables:");
        for var in &locals {
            println!(
                "  {} ({})",
                var.name,
                var.ty.as_ref().map_or("?", |t| &t.name)
            );
            if let Some(value) = &var.value {
                print_value(value, 4);
            }
        }
        println!();
    }

    if !globals.is_empty() {
        println!("Global variables:");
        for var in &globals {
            println!(
                "  {} ({})",
                var.name,
                var.ty.as_ref().map_or("?", |t| &t.name)
            );
            if let Some(value) = &var.value {
                print_value(value, 4);
            }
        }
    }

    Ok(())
}
