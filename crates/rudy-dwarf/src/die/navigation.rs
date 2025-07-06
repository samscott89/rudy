//! DWARF tree navigation and traversal functions

use gimli::UnitSectionOffset;
use itertools::Itertools;

use super::{
    unit::{get_unit_ref, UnitRef},
    utils::{file_entry_to_path, get_dwarf, to_range},
    CompilationUnitId,
};
use crate::{
    file::{DebugFile, SourceFile},
    DwarfDb,
};

/// Root compilation unit information
#[salsa::tracked]
pub struct Root<'db> {
    pub cu: CompilationUnitId,
    pub address_range: (u64, u64),
    #[returns(ref)]
    pub files: Vec<SourceFile>,
}

/// Get all root compilation units from a file
pub fn get_roots<'db>(
    db: &'db dyn DwarfDb,
    file: DebugFile,
) -> Vec<(UnitSectionOffset, UnitRef<'db>)> {
    let Some(dwarf) = get_dwarf(db, file.file(db)) else {
        return Default::default();
    };

    let mut roots = vec![];
    let mut units = dwarf.units();
    loop {
        let header = match units.next() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("Failed to parse unit: {e}");
                continue;
            }
        };
        let cu_offset = header.offset();
        let unit_ref = match get_unit_ref(db, file, cu_offset) {
            Some(unit_ref) => unit_ref,
            None => continue,
        };
        roots.push((cu_offset, unit_ref));
    }

    roots
}

/// Parse root compilation units with their metadata
#[salsa::tracked]
pub fn parse_roots<'db>(db: &'db dyn DwarfDb, file: DebugFile) -> Vec<Root<'db>> {
    let Some(dwarf) = get_dwarf(db, file.file(db)) else {
        return Default::default();
    };

    let mut roots = vec![];

    let mut units = dwarf.units();
    loop {
        let header = match units.next() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("Failed to parse unit: {e}");
                continue;
            }
        };
        let cu_offset = header.offset();
        let Some(unit_ref) = get_unit_ref(db, file, cu_offset) else {
            continue;
        };
        let addr_range = match unit_ref
            .unit_ranges()
            .map_err(anyhow::Error::from)
            .and_then(to_range)
        {
            Ok(Some(res)) => res,
            Ok(None) => {
                continue;
            }
            Err(e) => {
                tracing::error!("Failed to parse ranges for compilation unit: {e}");
                continue;
            }
        };
        let die = CompilationUnitId::new(file, cu_offset);

        let referenced_files = unit_ref
            .line_program
            .as_ref()
            .map(|lp| {
                lp.header()
                    .file_names()
                    .iter()
                    .flat_map(|f| {
                        let path = file_entry_to_path(db, f, &unit_ref)?;
                        Some(SourceFile::new(path))
                    })
                    .collect_vec()
            })
            .unwrap_or_default();

        roots.push(Root::new(db, die, addr_range, referenced_files));
    }

    roots
}
