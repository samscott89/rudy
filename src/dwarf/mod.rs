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
mod navigation;
mod resolution;
mod unit;
mod utils;

// Re-exports
pub use cu::CompilationUnitId;
pub use die::Die;
pub use expressions::resolve_data_location;
pub use index::{Index, IndexData, index_symbols, index_types};
pub use loader::{Dwarf, load};
pub use navigation::{address_in_entry, parse_roots};
pub use resolution::{
    // Function resolution
    Function,
    // Address resolution
    ResolvedLocation,
    // Variable resolution
    Variable,
    address_to_location,
    get_def,
    location_to_address,
    resolve_function,
    resolve_function_variables,
    resolve_type,
};
