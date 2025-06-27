//! Core DWARF entity types and their navigation methods

use gimli::UnitSectionOffset;

use crate::database::Db;
use crate::file::DebugFile;

use super::unit::UnitRef;

/// References a specific compilation unit in a DWARF file
#[salsa::interned(debug)]
#[derive(Ord, PartialOrd)]
pub struct CompilationUnitId<'db> {
    pub file: DebugFile,
    pub offset: UnitSectionOffset<usize>,
}

impl<'db> CompilationUnitId<'db> {
    pub fn unit_ref(&self, db: &'db dyn Db) -> Option<UnitRef<'db>> {
        super::unit::get_unit_ref(db, self.file(db), self.offset(db))
    }
}
