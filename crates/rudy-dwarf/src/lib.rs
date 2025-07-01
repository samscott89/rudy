//! DWARF debug information parsing and querying
//!
//! This crate provides functionality for parsing DWARF debug information
//! from object files and querying it through a salsa database.

// Sub-modules
pub mod address_tree;
mod cu;
mod die;
pub mod file;
mod index;
mod loader;
mod names;
mod navigation;
mod parser;
mod resolution;
pub mod symbols;
pub mod types;
mod unit;
mod utils;
mod visitor;

#[cfg(test)]
pub mod test_utils;

pub use gimli;

use std::path::{Path, PathBuf};

// Re-exports for the public API
pub use address_tree::{AddressTree, FunctionAddressInfo};
pub use cu::CompilationUnitId;
pub use die::Die;
pub use file::{Binary, DebugFile, File, SourceFile};
pub use index::{
    find_type_by_name, function_index, index_debug_file_sources, FunctionIndex, FunctionIndexEntry,
};
pub use loader::{load, Dwarf, DwarfReader};
pub use names::{RawSymbol, SymbolName, TypeName};
pub use resolution::{
    address_to_location,
    location_to_address,
    resolve_function_signature,
    resolve_function_variables,
    resolve_type_offset,
    FunctionSignature,
    // Address resolution
    ResolvedLocation,
    // Variable resolution
    Variable,
};
pub use unit::UnitRef;
pub use utils::get_string_attr;
pub use visitor::{walk_file, DieVisitor, DieWalker};

// Database trait that this crate requires
#[salsa::db]
pub trait DwarfDb: salsa::Database {
    /// Get source path remapping
    fn remap_path(&self, path: &Path) -> PathBuf {
        let mut path = path.to_path_buf();
        for (source, target) in self.get_source_map() {
            if let Ok(stripped) = path.strip_prefix(source) {
                tracing::debug!(
                    "Remapping {} from {} to {}",
                    path.display(),
                    source.display(),
                    target.display()
                );
                path = target.join(stripped);
            }
        }
        path
    }

    /// Get the source map for path remapping
    fn get_source_map(&self) -> &[(PathBuf, PathBuf)];
}
