//! Query functions for looking up debug information

use itertools::Itertools;

use crate::database::Db;
use crate::dwarf;
use crate::file::{Binary, SourceFile};
use crate::index;
use crate::typedef::TypeDef;
use crate::types::{Address, FunctionIndexEntry, NameId, Position};

#[salsa::tracked]
pub fn find_closest_match<'db>(
    db: &'db dyn Db,
    binary: Binary,
    function_name: NameId<'db>,
) -> Option<(NameId<'db>, FunctionIndexEntry<'db>)> {
    // check if exact name exists in index
    let index = index::build_index(db, binary);
    if let Some(entry) = index.data(db).function_name_to_die.get(&function_name) {
        return Some((function_name, *entry));
    }

    // otherwise, find the closest match by scanning the index
    let name = function_name.name(db);
    let module_prefix = function_name.path(db);

    index
        .data(db)
        .function_name_to_die
        .iter()
        .find_map(|(indexed_name, entry)| {
            if indexed_name.name(db) == name
                && indexed_name.path(db).ends_with(module_prefix.as_slice())
            {
                Some((*indexed_name, *entry))
            } else {
                None
            }
        })
}

#[salsa::tracked]
pub fn lookup_position<'db>(db: &'db dyn Db, binary: Binary, query: Position<'db>) -> Option<u64> {
    let file_name = query.file(db);
    let file = SourceFile::new(db, file_name);

    // find compilation units that cover the provided file
    let index = index::build_index(db, binary);
    let Some(cu_ids) = index.data(db).file_to_cu.get(&file) else {
        tracing::debug!(
            "no compilation units found for file: {} in index {:#?}",
            file.path(db),
            index.data(db).file_to_cu
        );
        return None;
    };

    if cu_ids.is_empty() {
        tracing::debug!("No compilation units found for file: {}", file.path(db));
        return None;
    }

    let mut closest_match: Option<u64> = None;
    let mut closest_line = u64::MAX;

    // find closest match to this line + column within the files
    for cu in cu_ids {
        let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu).copied() else {
            tracing::debug!("no base address found for {cu:?}");
            continue;
        };
        tracing::debug!("looking for matches in {cu:?}");
        if let Some((addr, distance)) = dwarf::location_to_address(db, *cu, query.clone()) {
            tracing::debug!("found match  {addr:#x} at distance {distance}");
            if distance < closest_line {
                tracing::debug!(
                    "base: {base_addr:#x} + addr: {addr:#x} = {:#x}",
                    base_addr + addr
                );
                closest_match = Some(base_addr + addr);
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

#[tracing::instrument(skip(db))]
#[salsa::tracked]
pub fn lookup_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: Address<'db>,
) -> Option<dwarf::ResolvedLocation<'db>> {
    let address = address.address(db);
    let index = index::build_index(db, binary);
    let cu_index = &index.data(db).address_range_to_cu;
    let range_start = cu_index.partition_point(|(start, _, _)| *start < address);
    let range = &cu_index[..range_start];
    if range.is_empty() {
        return None;
    }
    tracing::trace!("found {} CU in range", range.len());
    // iterate through the matching ranges in _reverse_ order
    // since we know all of the ranges match the address on the left
    // hand side of the range
    for (start, end, cu) in range.iter().rev() {
        if address > *end {
            tracing::trace!("{address:#x} after {end:#x} for {cu:?}");
            continue;
        };
        tracing::debug!("found CU: {cu:?} ({start:#x}, {end:#x})");

        let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu) else {
            tracing::trace!("no base address found for {cu:?}");
            continue;
        };

        let Some(relative_addr) = address.checked_sub(*base_addr) else {
            tracing::trace!("address {address:#x} is before base address {base_addr:#x}");
            continue;
        };

        if let Some(loc) = dwarf::address_to_location(db, *cu, relative_addr) {
            tracing::trace!("resolved location: {loc:#?}");
            return Some(loc);
        } else {
            tracing::debug!("location not found");
        }
    }
    None
}

#[salsa::tracked]
pub fn lookup_closest_function<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: Address<'db>,
) -> Option<FunctionIndexEntry<'db>> {
    let address = address.address(db);
    tracing::debug!("looking up function for address {address:#x}");
    let index = index::build_index(db, binary);
    let function_index = &index.data(db).address_range_to_function;

    let range_start = function_index.partition_point(|(start, _, _)| *start < address);
    let range = &function_index[..range_start];
    if range.is_empty() {
        tracing::warn!(
            "no function found in range: index: {function_index}\naddress: {address}\nrange: {range}",
            function_index = function_index
                .iter()
                .map(|(start, end, name)| format!("{start:#x}..{end:#x} ({})", name.as_path(db)))
                .join(",\n\t"),
            range = range
                .iter()
                .map(|(start, end, name)| format!("{start:#x}..{end:#x} ({})", name.as_path(db)))
                .join(",\n\t")
        );
        return None;
    }
    tracing::debug!("found {} functions in range", range.len());
    for (start, end, name) in range {
        if address < *start {
            tracing::error!("address {address:#x} is before start {start:#x}");
        }
        if address > *end {
            tracing::trace!("address {address:#x} is after end {end:#x}");
            continue;
        };
        tracing::trace!("function: {name:?} ({start:#x}, {end:#x})");
        let function = index.data(db).function_name_to_die.get(name)?;
        let die_id = function.die(db);
        let base_address = index.data(db).cu_to_base_addr.get(&die_id.cu(db))?;
        // do the more precise check using the exact ranges iterator
        // for the entry
        let relative_address = address - base_address;
        // check the precise address is in the ranges covered
        // by the function (in the case of non-contiguous ranges)
        if dwarf::address_in_entry(db, relative_address, die_id) {
            return Some(*function);
        } else {
            tracing::debug!("not in entry");
        }
    }
    None
}

#[salsa::tracked]
pub fn test_get_def<'db>(db: &'db dyn Db, binary: Binary) -> TypeDef<'db> {
    let index = index::build_index(db, binary);

    // find the STATIC_TEST_STRUCT global constants
    let (_, static_test_struct) = index
        .data(db)
        .symbol_name_to_die
        .iter()
        .find(|(name, _)| {
            let name = name.name(db);
            name.contains("STATIC_TEST_STRUCT")
        })
        .expect("should find test struct");

    // get its DIE entry + type
    dwarf::resolve_type(db, static_test_struct.die(db)).expect("could not get type")
}
