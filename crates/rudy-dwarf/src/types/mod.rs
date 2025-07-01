//! Type indexing and resolution logic

mod resolution;

// For now, we'll keep this simple until we fully split index.rs
// Type-related indexing functions will move here from index.rs

pub use resolution::{resolve_entry_type, resolve_type_offset, shallow_resolve_type};
