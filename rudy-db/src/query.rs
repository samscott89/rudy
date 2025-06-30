//! Query functions for looking up debug information

use crate::database::Db;
use crate::dwarf::{self, SymbolName};
use crate::file::Binary;
use crate::index::{self, find_all_by_address};
use crate::types::{Address, Position};

#[salsa::tracked]
pub fn lookup_position<'db>(db: &'db dyn Db, binary: Binary, query: Position<'db>) -> Option<u64> {
    let file = query.file(db);

    // find compilation units that cover the provided file
    let index = index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);
    let source_to_file = index.source_to_file(db);

    let mut closest_match: Option<u64> = None;
    let mut closest_line = u64::MAX;

    // find closest match to this line + column within the files
    for debug_file in source_to_file.get(&file)? {
        // let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu).copied() else {
        //     tracing::debug!("no base address found for {cu:?}");
        //     continue;
        // };
        // tracing::debug!("looking for matches in {cu:?}");
        if let Some((addr, distance)) = dwarf::location_to_address(db, *debug_file, query) {
            tracing::debug!("found match  {addr:#x} at distance {distance}");
            if distance < closest_line {
                // if we found a closer match, find the absolute address
                let matching_functions = symbol_index
                    .function_index(db, *debug_file)
                    .map(|fi| fi.by_relative_address(db).query_address(addr))
                    .unwrap_or_default();

                if matching_functions.is_empty() {
                    tracing::debug!(
                        "no functions found for address {addr:#x} in file {debug_file:?}"
                    );
                    continue;
                } else {
                    tracing::debug!(
                        "found {} functions for address {addr:#x} in file {debug_file:?}",
                        matching_functions.len()
                    );
                }

                for f in matching_functions {
                    let relative_start = f.start;
                    if let Some(abs_start) = symbol_index.get_function(&f.name).map(|s| s.address) {
                        let addr = addr + abs_start - relative_start;
                        tracing::debug!(
                            "found closest match at {addr:#x} for address {addr:#x} in file {debug_file:?}"
                        );
                        closest_match = Some(addr);
                        closest_line = distance;
                        break;
                    }
                }
            }
            if distance == 0 {
                // we found an exact match, so we can stop
                break;
            }
        }
    }

    // return the address
    closest_match
}

#[tracing::instrument(skip_all, fields(binary=binary.name(db), address=address.address(db)))]
#[salsa::tracked]
pub fn lookup_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: Address<'db>,
) -> Option<(SymbolName, dwarf::ResolvedLocation<'db>)> {
    let address = address.address(db);

    find_all_by_address(db, binary, address)
        .into_iter()
        .filter_map(|(relative_addr, cu, fai)| {
            tracing::debug!("looking up address {relative_addr:#x} in {cu:?} for function {fai:?}");
            dwarf::address_to_location(db, cu, relative_addr).map(|loc| (fai.name.clone(), loc))
        })
        .next()
}
