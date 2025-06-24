//! DWARF debug information parsing and querying
//!
//! This module provides functionality for parsing DWARF debug information
//! from object files and querying it through a salsa database.

// Sub-modules
mod cu;
mod die;
mod expressions;
mod index;
mod loader;
mod names;
mod navigation;
mod resolution;
mod unit;
mod utils;
mod visitor;

// Re-exports
pub use cu::CompilationUnitId;
pub use die::Die;
pub use expressions::resolve_data_location;
pub use index::{FileIndex, FunctionIndexEntry, index_debug_file_full, index_debug_file_sources};
pub use loader::{Dwarf, load};
pub use names::{ModuleName, RawSymbol, SymbolName, TypeName};
pub use resolution::{
    // Address resolution
    ResolvedLocation,
    // Variable resolution
    Variable,
    address_to_location,
    fully_resolve_type,
    location_to_address,
    resolve_function_variables,
    resolve_type_offset,
};
