//! Query functions for looking up debug information

use crate::database::Db;
use crate::index::{self, find_all_by_address};
use crate::types::Address;
use rudy_dwarf::{Binary, SymbolName};

#[salsa::tracked]
pub fn lookup_position<'db>(
    db: &'db dyn Db,
    binary: Binary,
    query: rudy_dwarf::file::SourceLocation<'db>,
) -> Option<u64> {
    let file = query.file(db);

    // find compilation units that cover the provided file
    let index = index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);
    let source_to_file = index.source_to_file(db);

    let mut closest_match: Option<u64> = None;
    let mut closest_line = u64::MAX;

    // find closest match to this line + column within the files
    for debug_file in source_to_file.get(&file)? {
        let Some(function_index) = symbol_index.function_index(db, *debug_file) else {
            continue;
        };
        // let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu).copied() else {
        //     tracing::debug!("no base address found for {cu:?}");
        //     continue;
        // };
        // tracing::debug!("looking for matches in {cu:?}");
        if let Some((addr, distance)) = function_index.location_to_address(db, *debug_file, query) {
            tracing::debug!("found match  {addr:#x} at distance {distance}");
            if distance < closest_line {
                tracing::debug!(
                    "found closest match at {addr:#x} for address {addr:#x} in file {debug_file:?}"
                );
                closest_match = Some(addr);
                closest_line = distance;
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
) -> Option<(SymbolName, rudy_dwarf::file::SourceLocation<'db>)> {
    let address = address.address(db);

    let mut results = find_all_by_address(db, binary, address);

    if results.len() > 1 {
        tracing::warn!(
            "Multiple results found for address {address:#x} in binary {binary:?}. Returning the first result."
        );
    }
    // pop a single result off
    // TODO: handle multiple results better
    results.pop()
}
