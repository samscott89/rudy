//! Type indexing and resolution logic

mod index;
mod resolution;

pub use index::{find_type_by_name, get_die_typename, TypeIndexEntry};
pub use resolution::{resolve_entry_type, resolve_type_offset, shallow_resolve_type};

use crate::Die;

pub type DieTypeDefinition = rudy_types::TypeDefinition<Die>;
pub type DieLayout = rudy_types::Layout<Die>;

impl rudy_types::Location for Die {}
