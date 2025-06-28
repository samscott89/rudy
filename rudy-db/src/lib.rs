//! # Rudy DB
//!
//! A user-friendly library for interacting with debugging information of Rust compiled artifacts using DWARF.
//!
//! This library provides lazy evaluation and incremental recomputation via salsa for use in
//! long-running processes like debuggers.
//!
//! ## Basic Usage
//!
//! ```no_run
//! use rudy_db::{DebugDb, DebugInfo};
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
mod function_discovery;
mod index;
mod outputs;
mod query;
mod synthetic_methods;
mod types;

// re-exports
pub use data::DataResolver;
pub use database::DebugDatabaseImpl as DebugDb;
pub use debug_info::DebugInfo;
pub use function_discovery::{DiscoveredFunction, SelfType};
pub use index::symbols::Symbol;
pub use outputs::{ResolvedAddress, ResolvedLocation, Type, Value, Variable, VariableInfo};
pub use rudy_types::TypeLayout;
pub use synthetic_methods::{SyntheticMethod, evaluate_synthetic_method, get_synthetic_methods};
