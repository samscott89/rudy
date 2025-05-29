//! Core DWARF entity types and their navigation methods

use gimli::UnitSectionOffset;

use crate::database::Db;
use crate::file::FileId;

use super::unit::UnitRef;

/// References a specific compilation unit in a DWARF file
#[salsa::interned]
pub struct CompilationUnitId<'db> {
    pub file: FileId<'db>,
    pub offset: UnitSectionOffset<usize>,
}

impl<'db> CompilationUnitId<'db> {
    pub fn unit_ref(&self, db: &'db dyn Db) -> Option<UnitRef<'db>> {
        super::unit::get_unit_ref(db, self.file(db), self.offset(db))
    }

    pub fn as_path_ref(&self, db: &'db dyn Db) -> String {
        let path = self.file(db).full_path(db);
        let cu_offset = match self.offset(db) {
            UnitSectionOffset::DebugInfoOffset(debug_info_offset) => debug_info_offset.0,
            UnitSectionOffset::DebugTypesOffset(debug_types_offset) => debug_types_offset.0,
        };
        format!("{path}:{cu_offset:#x}")
    }
}
