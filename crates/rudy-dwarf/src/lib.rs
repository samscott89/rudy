//! DWARF debug information parsing and querying
//!
//! This crate provides functionality for parsing DWARF debug information
//! from object files and querying it through a salsa database.

// Public modules - clean API for users
pub mod address;
pub mod die;
pub mod error;
pub mod expressions;
pub mod file;
pub mod function;
pub mod index;
pub mod parser;
pub mod symbols;
pub mod types;
pub mod visitor;

// Test utilities
#[cfg(test)]
pub mod test_utils;

// Essential re-exports for convenience
pub use gimli;

// Core types that most users will need
pub use die::Die;
pub use error::Error;
pub use file::{Binary, DebugFile, SourceFile};
pub use symbols::{SymbolName, TypeName};

use std::path::{Path, PathBuf};

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
