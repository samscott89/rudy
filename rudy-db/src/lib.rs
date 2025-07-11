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
//!     if let Ok(Some(location)) = debug_info.address_to_location(0x12345) {
//!         println!("Address 0x12345 is at {}:{}", location.file, location.line);
//!     }
//!     
//!     Ok(())
//! }
//! ```

mod data;
mod database;
mod debug_info;
mod function_discovery;
mod index;
mod outputs;
mod query;
mod synthetic_methods;
#[cfg(test)]
pub mod test_utils;

// crate re-exports
pub use rudy_dwarf;
pub use rudy_types;

// common type re-exports
pub use data::DataResolver;
pub use database::DebugDatabaseImpl as DebugDb;
pub use debug_info::DebugInfo;
pub use outputs::{
    DiscoveredFunction, DiscoveredMethod, FunctionParameter, ResolvedAddress, ResolvedLocation,
    Type, TypedPointer, Value, Variable, VariableInfo,
};
pub use rudy_dwarf::function::SelfType;
pub use synthetic_methods::{SyntheticMethod, evaluate_synthetic_method, get_synthetic_methods};
