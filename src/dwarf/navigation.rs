//! DWARF tree navigation and traversal functions

use gimli::{Unit, UnitSectionOffset};
use itertools::Itertools;

use super::{
    CompilationUnitId, Die,
    loader::DwarfReader,
    utils::{file_entry_to_path, to_range},
};
use crate::database::Db;
use crate::file::{FileId, SourceFile};

/// Root compilation unit information
#[salsa::tracked]
pub struct Root<'db> {
    pub cu: CompilationUnitId<'db>,
    pub address_range: (u64, u64),
    #[return_ref]
    pub files: Vec<SourceFile<'db>>,
}

/// Get all root compilation units from a file
pub fn get_roots<'db>(
    db: &'db dyn Db,
    file_id: FileId<'db>,
) -> Vec<(UnitSectionOffset, Unit<DwarfReader>)> {
    let Some(dwarf) = db.get_file(file_id).and_then(|f| f.dwarf()) else {
        return Default::default();
    };

    let mut roots = vec![];
    let mut units = dwarf.units();
    loop {
        let header = match units.next() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                db.report_critical(format!("Failed to parse unit: {e}"));
                continue;
            }
        };
        let cu_offset = header.offset();
        let Some(unit) = dwarf
            .unit(header)
            .inspect_err(|e| {
                db.report_critical(format!("Failed to parse unit: {e}"));
            })
            .ok()
        else {
            continue;
        };

        roots.push((cu_offset, unit));
    }

    roots
}

/// Parse root compilation units with their metadata
#[salsa::tracked]
pub fn parse_roots<'db>(db: &'db dyn Db, file_id: FileId<'db>) -> Vec<Root<'db>> {
    let Some(file) = db.get_file(file_id) else {
        return Default::default();
    };
    let Some(dwarf) = file.dwarf() else {
        return Default::default();
    };

    let mut roots = vec![];

    let mut units = dwarf.units();
    loop {
        let header = match units.next() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                db.report_critical(format!("Failed to parse unit: {e}"));
                continue;
            }
        };
        let cu_offset = header.offset();
        let Some(unit) = dwarf
            .unit(header)
            .inspect_err(|e| {
                db.report_critical(format!("Failed to parse unit: {e}"));
            })
            .ok()
        else {
            continue;
        };
        let unit_ref = unit.unit_ref(dwarf);
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
                db.report_critical(format!("Failed to parse ranges for compilation unit: {e}"));
                continue;
            }
        };
        let die = CompilationUnitId::new(db, file_id, cu_offset);

        let referenced_files = unit_ref
            .line_program
            .as_ref()
            .map(|lp| {
                lp.header()
                    .file_names()
                    .iter()
                    .flat_map(|f| {
                        let path = file_entry_to_path(f, &unit_ref)?;
                        Some(SourceFile::new(db, path))
                    })
                    .collect_vec()
            })
            .unwrap_or_default();

        roots.push(Root::new(db, die, addr_range, referenced_files));
    }

    roots
}

/// Check if an address is within a DIE entry's ranges
pub fn address_in_entry<'db>(db: &'db dyn Db, relative_address: u64, die: Die<'db>) -> bool {
    die.with_entry_and_unit(db, |entry, unit_ref| {
        let mut ranges = match unit_ref.die_ranges(&entry) {
            Ok(ranges) => ranges,
            Err(e) => {
                db.report_critical(format!("Failed to get ranges: {e}"));
                return false;
            }
        };
        loop {
            match ranges.next() {
                Ok(Some(range)) => {
                    tracing::debug!("checking range ({:#x}, {:#x})", range.begin, range.end);
                    if relative_address >= range.begin && relative_address <= range.end {
                        return true;
                    }
                }
                // we've checked all range, and none matched
                Ok(None) => return false,
                Err(e) => {
                    db.report_critical(format!("Failed to parse ranges: {e}"));
                    continue;
                }
            }
        }
    })
    .unwrap_or(false)
}
