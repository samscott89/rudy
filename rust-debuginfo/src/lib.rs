//! # rust-debuginfo
//!
//! A user-friendly library for interacting with debugging information of Rust compiled artifacts using DWARF.
//!
//! This library provides lazy evaluation and incremental recomputation via salsa for use in
//! long-running processes like debuggers.
//!
//! ## Basic Usage
//!
//! ```no_run
//! use rust_debuginfo::{DebugDb, DebugInfo};
//! use anyhow::Result;
//!
//! fn main() -> Result<()> {
//!     // Create a new database
//!     let mut db = DebugDb::new();
//!     
//!     // Create a DebugInfo instance for your binary
//!     let debug_info = DebugInfo::new(&db, "path/to/binary")?;
//!     
//!     // Resolve an address to source location
//!     if let Some(location) = debug_info.address_to_line(0x12345) {
//!         println!("Address 0x12345 is at {}:{}", location.file, location.line);
//!     }
//!     
//!     Ok(())
//! }
//! ```

mod address_tree;
mod data;
mod database;
mod debug_info;
mod dwarf;
mod file;
mod index;
mod outputs;
mod query;
mod typedef;
mod types;

// re-exports
pub use data::DataResolver;
pub use database::DebugDatabaseImpl as DebugDb;
pub use debug_info::DebugInfo;
pub use outputs::{ResolvedAddress, ResolvedLocation, Type, Value, Variable};
pub use typedef::TypeDef;
