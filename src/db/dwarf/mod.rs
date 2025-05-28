//! DWARF debug information parsing and querying
//!
//! This module provides functionality for parsing DWARF debug information
//! from object files and querying it through a salsa database.

// Sub-modules
mod entities;
mod expressions;
mod index;
mod loader;
mod navigation;
mod resolution;
mod unit;
mod utils;

// Re-exports
pub use entities::{CompilationUnitId, DieEntryId};
pub use expressions::resolve_data_location;
pub use index::{Index, IndexData, index_symbols, index_types};
pub use loader::{load, Dwarf};
pub use navigation::{address_in_entry, parse_roots};
pub use resolution::{
    // Address resolution
    ResolvedLocation, address_to_location, location_to_address,
    // Function resolution
    Function, resolve_function,
    // Type resolution
    resolve_type_offset,
    // Variable resolution
    Variable, resolve_function_variables,
};