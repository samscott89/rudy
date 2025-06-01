//! DWARF indexing functionality for fast lookups

use std::collections::BTreeMap;

use super::{CompilationUnitId, Die};
use crate::file::SourceFile;
use crate::types::{FunctionIndexEntry, NameId, SymbolIndexEntry, TypeIndexEntry};

mod symbols;
mod types;

pub use symbols::index_symbols;
pub use types::index_types;

/// Pre-computed index for fast lookups
#[salsa::tracked(debug)]
pub struct Index<'db> {
    #[returns(ref)]
    pub data: IndexData<'db>,
}

/// Index data structure containing all mappings
#[derive(Default, Hash, PartialEq)]
pub struct IndexData<'db> {
    pub function_name_to_die: BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>,
    pub symbol_name_to_die: BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>,
    pub type_name_to_die: BTreeMap<NameId<'db>, TypeIndexEntry<'db>>,
    pub die_to_type_name: BTreeMap<Die<'db>, NameId<'db>>,
    pub cu_to_base_addr: BTreeMap<CompilationUnitId<'db>, u64>,
    pub address_range_to_cu: Vec<(u64, u64, CompilationUnitId<'db>)>,
    pub address_range_to_function: Vec<(u64, u64, NameId<'db>)>,
    pub file_to_cu: BTreeMap<SourceFile<'db>, Vec<CompilationUnitId<'db>>>,
}

unsafe impl salsa::Update for IndexData<'_> {
    unsafe fn maybe_update(_: *mut Self, _: Self) -> bool {
        // IndexData should never change after creation
        todo!()
    }
}
