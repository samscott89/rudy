//! Query functions for looking up debug information

use crate::database::Db;
use crate::dwarf::{self};
use crate::file::Binary;
use crate::index::{self, find_all_by_address};
use crate::typedef::TypeDef;
use crate::types::{Address, NameId, Position};

#[salsa::tracked]
pub fn lookup_position<'db>(db: &'db dyn Db, binary: Binary, query: Position<'db>) -> Option<u64> {
    let file = query.file(db);

    // find compilation units that cover the provided file
    let index = index::debug_index(db, binary).data(db);

    let mut closest_match: Option<u64> = None;
    let mut closest_line = u64::MAX;

    // find closest match to this line + column within the files
    for debug_file in index.source_to_file.get(&file)? {
        // let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu).copied() else {
        //     tracing::debug!("no base address found for {cu:?}");
        //     continue;
        // };
        // tracing::debug!("looking for matches in {cu:?}");
        if let Some((addr, distance)) = dwarf::location_to_address(db, *debug_file, query.clone()) {
            tracing::debug!("found match  {addr:#x} at distance {distance}");
            if distance < closest_line {
                // if we found a closer match, find the absolute address
                let matching_functions = dwarf::debug_file_index(db, *debug_file)
                    .data(db)
                    .function_addresses
                    .query_address(addr);

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
                    if let Some(abs_start) = index.base_address.get(&f.name) {
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

#[tracing::instrument(skip(db))]
#[salsa::tracked]
pub fn lookup_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: Address<'db>,
) -> Option<dwarf::ResolvedLocation<'db>> {
    let address = address.address(db);

    find_all_by_address(db, binary, address)
        .into_iter()
        .filter_map(|fai| {
            let indexed_function = dwarf::debug_file_index(db, fai.file)
                .data(db)
                .functions
                .get(&fai.name)?;
            let cu = indexed_function.declaration_die.cu(db);

            // offset into the functino
            let function_offset = address - fai.start;
            let relative_address = indexed_function
                .relative_address_range
                .unwrap_or_default() // TODO: Fix
                .0
                + function_offset;

            dwarf::address_to_location(db, cu, relative_address)
        })
        .next()
}

#[tracing::instrument(skip(db))]
#[salsa::tracked]
pub fn lookup_closest_function<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: Address<'db>,
) -> Option<NameId<'db>> {
    let address = address.address(db);

    find_all_by_address(db, binary, address)
        .into_iter()
        .map(|fai| fai.name.clone())
        .next()
}

#[salsa::tracked]
pub fn test_get_def<'db>(db: &'db dyn Db, binary: Binary) -> TypeDef<'db> {
    let index = index::debug_index(db, binary);

    // find the STATIC_TEST_STRUCT global constants
    let static_test_struct = index
        .data(db)
        .name_to_file
        .iter()
        .find_map(|(name, f)| {
            let struct_name = name.name(db);
            if struct_name.contains("STATIC_TEST_STRUCT") {
                let indexed_file = dwarf::debug_file_index(db, *f);
                indexed_file.data(db).types.get(name).cloned()
            } else {
                None
            }
        })
        .expect("should find test struct");

    // get its DIE entry + type
    dwarf::resolve_type(db, static_test_struct.die(db)).expect("could not get type")
}
