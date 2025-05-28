//! DWARF indexing functionality for fast lookups

use std::collections::BTreeMap;

use crate::types::{NameId, FunctionIndexEntry, SymbolIndexEntry, TypeIndexEntry};
use crate::file::SourceFile;
use super::entities::{CompilationUnitId, DieEntryId};

mod symbols;
mod types;

pub use symbols::index_symbols;
pub use types::index_types;

/// Pre-computed index for fast lookups
#[salsa::tracked]
pub struct Index<'db> {
    #[return_ref]
    pub data: IndexData<'db>,
}

/// Index data structure containing all mappings
#[derive(Default, Hash, PartialEq, salsa::Update)]
pub struct IndexData<'db> {
    pub function_name_to_die: BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>,
    pub symbol_name_to_die: BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>,
    pub type_name_to_die: BTreeMap<NameId<'db>, TypeIndexEntry<'db>>,
    pub die_to_type_name: BTreeMap<DieEntryId<'db>, NameId<'db>>,
    pub cu_to_base_addr: BTreeMap<CompilationUnitId<'db>, u64>,
    pub address_range_to_cu: Vec<(u64, u64, CompilationUnitId<'db>)>,
    pub address_range_to_function: Vec<(u64, u64, NameId<'db>)>,
    pub file_to_cu: BTreeMap<SourceFile<'db>, Vec<CompilationUnitId<'db>>>,
}