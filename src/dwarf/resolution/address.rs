//! Address to location and location to address resolution

use crate::database::Db;
use crate::file::SourceFile;
use crate::dwarf::{
    entities::CompilationUnitId,
    utils::file_entry_to_path,
};

/// Resolved source location information
#[salsa::interned]
pub struct ResolvedLocation<'db> {
    pub file: SourceFile<'db>,
    pub line: u64,
    pub column: Option<u64>,
}

/// Convert an address to source location within a compilation unit
pub fn address_to_location<'db>(
    db: &'db dyn Db,
    cu: CompilationUnitId<'db>,
    relative_address: u64,
) -> Option<ResolvedLocation<'db>> {
    let unit_ref = cu.unit_ref(db)?;

    let Some(line_program) = unit_ref.line_program.clone() else {
        return None;
    };

    let header = line_program.header();

    let mut rows = line_program.clone().rows();
    while let Some((_, row)) = rows.next_row().ok()? {
        if row.end_sequence() {
            continue;
        }

        // TODO(Sam): deal with non-exact matches?
        if row.address() == relative_address {
            let line = match row.line() {
                Some(l) => l.get(),
                None => {
                    tracing::debug!("no source line known for address {relative_address:#x}",);
                    continue;
                }
            };
            let column = match row.column() {
                gimli::ColumnType::LeftEdge => None,
                gimli::ColumnType::Column(non_zero) => Some(non_zero.get()),
            };
            let file = match row.file(header) {
                Some(file) => {
                    let path = file_entry_to_path(file, &unit_ref)?;
                    SourceFile::new(db, path)
                }
                None => {
                    tracing::debug!("no source file known for address {relative_address:#x}",);
                    continue;
                }
            };
            tracing::debug!("found line {line} at address {relative_address:#x}");
            return Some(ResolvedLocation::new(db, file, line, column));
        }
    }

    None
}

/// Convert a source location to an address within a compilation unit
pub fn location_to_address<'db>(
    db: &'db dyn Db,
    cu: CompilationUnitId<'db>,
    query: crate::types::Position,
) -> Option<(u64, u64)> {
    let file = query.file(db);
    let (dir, file) = file.rsplit_once("/")?;
    let target_line = query.line(db);
    let _target_column = query.column(db);

    let unit_ref = cu.unit_ref(db)?;

    let Some(line_program) = unit_ref.line_program.clone() else {
        return None;
    };

    let header = line_program.header();

    let Some(target_file_idx) =
        header
            .clone()
            .file_names()
            .iter()
            .enumerate()
            .find_map(|(idx, f)| {
                let dir_bytes = unit_ref.attr_string(f.directory(header)?).ok()?;
                let path_bytes = unit_ref.attr_string(f.path_name()).ok()?;
                if &*dir_bytes == dir.as_bytes() && &*path_bytes == file.as_bytes() {
                    // files start at index 1
                    Some(idx as u64 + 1)
                } else {
                    None
                }
            })
    else {
        tracing::warn!("could not find target file {file} in line program");
        return None;
    };

    tracing::debug!("searching for target file: {target_file_idx}");

    let mut rows = line_program.clone().rows();

    let mut min_line_distance = u64::MAX;
    // let mut min_col_distance = u64::MAX;
    let mut closest_address: Option<u64> = None;

    loop {
        match rows.next_row() {
            Ok(Some((_, row))) => {
                if row.end_sequence() {
                    continue;
                }

                if row.file_index() == target_file_idx {
                    let Some(line) = row.line() else {
                        tracing::debug!("no line info");
                        continue;
                    };

                    let Some(line_diff) = line.get().checked_sub(target_line) else {
                        // line is _before_ the target line -- skip it
                        continue;
                    };
                    if closest_address.is_none() || line_diff < min_line_distance {
                        closest_address = Some(row.address());
                        min_line_distance = line_diff;
                        if line_diff == 0 {
                            // we found an exact match -- let's just return immediately
                            return Some((row.address(), 0));
                        }
                    }
                } else {
                    tracing::debug!("row in the wrong file");
                }
            }
            Ok(None) => break,
            Err(e) => {
                db.report_critical(format!("Failed to parse line program: {e}"));
                continue;
            }
        }
    }

    closest_address.map(|addr| (addr, min_line_distance))
}