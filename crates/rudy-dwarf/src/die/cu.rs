//! Core DWARF entity types and their navigation methods

use gimli::UnitSectionOffset;

use super::unit::UnitRef;
use crate::{
    die::utils::{get_lang_attr, pretty_print_die_entry},
    file::{DebugFile, RawDie},
    DwarfDb,
};

/// References a specific compilation unit in a DWARF file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, salsa::Update)]
pub struct CompilationUnitId {
    pub(crate) file: DebugFile,
    pub(crate) offset: UnitSectionOffset<usize>,
}

impl CompilationUnitId {
    pub fn new(file: DebugFile, offset: UnitSectionOffset<usize>) -> Self {
        Self { file, offset }
    }

    pub fn unit_ref<'db>(&self, db: &'db dyn DwarfDb) -> Option<UnitRef<'db>> {
        super::unit::get_unit_ref(db, self.file, self.offset)
    }
}

pub fn is_rust_cu(root: &RawDie<'_>, unit_ref: &UnitRef<'_>) -> bool {
    match get_lang_attr(root, unit_ref) {
        Ok(Some(lang)) if lang == gimli::DW_LANG_Rust => {
            // this is a Rust file, we can index it
            true
        }
        Ok(_) => {
            // not a rust file / language not found
            tracing::debug!(
                "skipping non-Rust compilation unit: {}",
                pretty_print_die_entry(root, unit_ref)
            );
            false
        }
        Err(e) => {
            tracing::error!(
                "could not get language of compilation unit: {e}: \n{}",
                pretty_print_die_entry(root, unit_ref),
            );
            false
        }
    }
}
