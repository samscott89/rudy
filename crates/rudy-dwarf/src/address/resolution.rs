//! Address to location and location to address resolution

use itertools::Itertools;

use crate::die::file_entry_to_path;
use crate::die::navigation::parse_roots;
use crate::file::{DebugFile, SourceFile, SourceLocation};
use crate::index::{FunctionData, FunctionIndex};
use crate::DwarfDb;

/// Convert an address to source location within a compilation unit
///
/// This function takes in the `address` to find, as well as
/// the function data for the function that contains the address.
pub fn address_to_location<'db>(
    db: &'db dyn DwarfDb,
    relative_address: u64,
    data: &FunctionData<'db>,
) -> Option<SourceLocation<'db>> {
    let unit_ref = data
        .declaration_die
        .unit_ref(db)
        .expect("declaration die should have a unit reference");

    let line_program = unit_ref.line_program.clone()?;

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
                    let path = file_entry_to_path(db, file, &unit_ref)?;
                    SourceFile::new(db, path)
                }
                None => {
                    tracing::debug!("no source file known for address {relative_address:#x}",);
                    continue;
                }
            };
            tracing::debug!("found line {line} at address {relative_address:#x}");
            return Some(SourceLocation::new(db, file, line, column));
        }
    }

    None
}

/// Convert a source location to an address within a compilation unit
///
/// Returns an _absolute_ address by using the known function address information
pub fn location_to_address(
    db: &dyn DwarfDb,
    debug_file: DebugFile,
    function_index: &FunctionIndex<'_>,
    query: SourceLocation,
) -> Option<(u64, u64)> {
    let file = query.file(db);
    let file = file.path(db);
    // let (dir, file) = file.rsplit_once("/")?;
    let target_line = query.line(db);
    let _target_column = query.column(db);

    let mut min_line_distance = u64::MAX;
    // let mut min_col_distance = u64::MAX;
    let mut closest_address: Option<u64> = None;

    tracing::info!(
        "searching for `{}:{target_line}` in `{}`",
        file.display(),
        debug_file.name(db)
    );

    for root in parse_roots(db, debug_file) {
        if !root.files(db).contains(&query.file(db)) {
            tracing::debug!(
                "skipping root compilation unit {:#x} -- no matching file for `{}`",
                root.cu(db).offset(db).as_debug_info_offset().unwrap().0,
                file.display()
            );
            continue;
        }

        let cu = root.cu(db);
        let Some(unit_ref) = cu.unit_ref(db) else {
            tracing::debug!(
                "skipping root compilation unit {:#x} -- no unit reference",
                cu.offset(db).as_debug_info_offset().unwrap().0
            );
            continue;
        };

        let line_program = unit_ref.line_program.clone()?;

        let header = line_program.header();

        let Some(target_file_idx) =
            header
                .clone()
                .file_names()
                .iter()
                .enumerate()
                .find_map(|(idx, f)| {
                    let Some(file_path) = file_entry_to_path(db, f, &unit_ref) else {
                        tracing::debug!("failed to convert file entry to path");
                        return None;
                    };
                    tracing::trace!(
                        "checking file `{}` against target `{}`",
                        file_path.display(),
                        file.display()
                    );
                    if &file_path == file {
                        Some(idx as u64 + 1) // +1 because file indices are 1-based
                    } else {
                        None
                    }
                })
        else {
            tracing::trace!(
                "could not find target file `{}` in line program for {:#x} in file {}",
                file.display(),
                cu.offset(db).as_debug_info_offset().unwrap().0,
                debug_file.name(db)
            );
            continue;
        };

        tracing::debug!("searching for rows with target file: {target_file_idx}");

        let mut rows = line_program.clone().rows();

        loop {
            match rows.next_row() {
                Ok(Some((_, row))) => {
                    if row.end_sequence() {
                        continue;
                    }

                    if row.file_index() == target_file_idx {
                        let address = row.address();
                        let Some(line) = row.line() else {
                            tracing::trace!("no line info: {address:#x}");
                            continue;
                        };

                        let Some(line_diff) = line.get().checked_sub(target_line) else {
                            // line is _before_ the target line -- skip it
                            continue;
                        };
                        if closest_address.is_none() || line_diff < min_line_distance {
                            // lets attempt to find the function that contains this address
                            let addresses = function_index.by_address(db).query_address(
                                address,
                                // we have relative addresses in the line program,
                                false,
                            );

                            let absolute_address = match &addresses[..] {
                                [] => {
                                    tracing::warn!(
                                        "location resolves to address: {address:#x}, but function not found",
                                    );
                                    continue;
                                }
                                [function] => {
                                    tracing::debug!(
                                        "location resolves to function: {}",
                                        function.name
                                    );
                                    function.absolute_start + address - function.relative_start
                                }
                                [function, rest @ ..] => {
                                    tracing::debug!(
                                        "location resolves to multiple functions: {} and {}",
                                        function.name,
                                        rest.iter().map(|f| &f.name).join(", ")
                                    );
                                    // we can just use the first one for now
                                    function.absolute_start + address - function.relative_start
                                }
                            };

                            closest_address = Some(absolute_address);
                            min_line_distance = line_diff;
                            if line_diff == 0 {
                                // we found an exact match -- let's just return immediately
                                return Some((absolute_address, 0));
                            }
                        }
                    } else {
                        tracing::trace!("row in the wrong file");
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::error!("Failed to parse line program: {e}");
                    continue;
                }
            }
        }
    }
    closest_address.map(|addr| (addr, min_line_distance))
}
