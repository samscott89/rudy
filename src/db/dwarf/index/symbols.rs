//! Symbol indexing for functions and variables

use std::collections::{BTreeMap, btree_map::Entry};

use crate::db::dwarf::{
    entities::{CompilationUnitId, DieEntryId},
    navigation::get_roots,
    utils::{get_string_attr_raw, pretty_print_die_entry, to_range},
};
use crate::db::{Db, FileId, FunctionIndexEntry, NameId, SymbolIndexEntry};

#[derive(Default)]
pub struct FileIndexEntries<'db> {
    /// map of function name to debugging entry
    pub function_name_to_die: BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>,
    /// map of symbol name to debugging entry
    pub symbol_name_to_die: BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>,
    /// list of function names with their address ranges
    pub address_range_to_function: Vec<(u64, u64, NameId<'db>)>,
    /// map of base address to compilation unit
    pub cu_to_base_addr: BTreeMap<CompilationUnitId<'db>, u64>,
}

pub fn index_symbols<'db>(
    db: &'db dyn Db,
    file_id: FileId<'db>,
    mut symbols: BTreeMap<Vec<u8>, (u64, NameId<'db>)>,
) -> FileIndexEntries<'db> {
    let Some(dwarf) = db.get_file(file_id).and_then(|f| f.dwarf()) else {
        return Default::default();
    };

    tracing::debug!("indexing symbols for {}", file_id.full_path(db));

    let mut function_name_to_die = BTreeMap::new();
    let mut symbol_name_to_die = BTreeMap::new();
    let mut address_range_to_function = Vec::new();
    let mut cu_to_address = BTreeMap::new();

    let roots = get_roots(db, file_id);

    for (cu_offset, unit) in roots {
        let cu = CompilationUnitId::new(db, file_id, cu_offset);
        let unit_ref = unit.unit_ref(dwarf);

        let Some(mut tree) = unit.entries_tree(None).ok() else {
            continue;
        };
        let Some(root) = tree.root().ok() else {
            continue;
        };
        let root = root.entry();
        match root.attr_value(gimli::DW_AT_language) {
            Ok(Some(gimli::AttributeValue::Language(lang))) => {
                if lang != gimli::DW_LANG_Rust {
                    // not a Rust file -- continue
                    continue;
                }
            }
            _ => {
                tracing::error!(
                    "could not get language of compilation unit: {}",
                    pretty_print_die_entry(root, &unit_ref)
                );
                db.report_critical(format!("could not get language of compilation unit"));
            }
        }

        let mut entries = unit.entries();
        let mut recurse = true;

        loop {
            let die = if recurse {
                match entries.next_dfs() {
                    Ok(Some((_, die))) => die,
                    Ok(None) => break,
                    Err(e) => {
                        db.report_critical(format!("Failed to get entry: {e}"));
                        continue;
                    }
                }
            } else {
                // continue recursing
                recurse = true;
                match entries.next_sibling() {
                    Ok(Some(die)) => die,
                    Ok(None) => {
                        // we've run out of siblings, but we might still have
                        // another parent (which `next_dfs` will check)
                        continue;
                    }
                    Err(e) => {
                        db.report_critical(format!("Failed to get entry: {e}"));
                        continue;
                    }
                }
            };
            // is a function
            match die.tag() {
                gimli::DW_TAG_subprogram => {
                    // skip if it references some other type?
                    if die
                        .attr_value(gimli::DW_AT_specification)
                        .is_ok_and(|v| v.is_some())
                    {
                        continue;
                    }

                    if die
                        .attr_value(gimli::DW_AT_abstract_origin)
                        .is_ok_and(|v| v.is_some())
                    {
                        continue;
                    }

                    if die
                        .attr_value(gimli::DW_AT_external)
                        .is_ok_and(|v| v.is_some())
                    {
                        // external symbol -- skip
                        continue;
                    }
                    if die
                        .attr_value(gimli::DW_AT_prototyped)
                        .is_ok_and(|v| v.is_some())
                    {
                        // external symbol -- skip
                        continue;
                    }

                    // TODO: future cases to handle:
                    // 1. DW_AT_declaration = Flag(true)
                    //    - we have an _incomplete_ declaration of a function
                    //      and need to find what completes it
                    // 2. DW_AT_specification = Flag(true)
                    //    - we have a function that is defined in another
                    //      compilation unit, and we need to find that unit
                    // 3. DW_AT_inline = Inline(DW_INL_inlined)
                    //    - An inlined function
                    let linkage_name_bytes =
                        match get_string_attr_raw(die, gimli::DW_AT_linkage_name, &unit_ref) {
                            Ok(Some(name)) => name,
                            Ok(None) => {
                                let entry = pretty_print_die_entry(die, &unit_ref);
                                tracing::error!(
                                    "No linkage name attribute: \n{entry} in {}",
                                    file_id.full_path(db)
                                );
                                db.report_critical(format!("no linkage name attribute?"));
                                continue;
                            }
                            Err(e) => {
                                db.report_critical(format!(
                                    "Failed to get linkage name attribute: {e}"
                                ));
                                continue;
                            }
                        };

                    // find the name in the functions map
                    if let Some((absolute_function_addr, name)) =
                        symbols.remove(&*linkage_name_bytes)
                    {
                        match unit_ref
                            .die_ranges(die)
                            .map_err(anyhow::Error::from)
                            .and_then(to_range)
                        {
                            Ok(Some((start, end))) => {
                                // if we haven't yet set it, we know the
                                // start of the absolute text section now
                                match cu_to_address.entry(cu) {
                                    Entry::Vacant(e) => {
                                        if let Some(base_addr) =
                                            absolute_function_addr.checked_sub(start)
                                        {
                                            e.insert(base_addr);
                                        } else {
                                            db.report_critical(format!("start is greater than absolute function address: {start:#x} > {absolute_function_addr:#x} for {}", name.as_path(db)));
                                        }
                                    }
                                    Entry::Occupied(_) => {
                                        // nothing to do
                                    }
                                }

                                // convert to absolute addresses and push into vec
                                let abs_start = absolute_function_addr;
                                let abs_end = absolute_function_addr + (end - start);
                                address_range_to_function.push((abs_start, abs_end, name));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                db.report_critical(format!("Failed to get ranges: {e}"));
                                continue;
                            }
                        };

                        let die_entry = DieEntryId::new(db, file_id, cu_offset, die.offset());
                        tracing::debug!("got function info for {}", name.as_path(db),);
                        function_name_to_die.insert(name, FunctionIndexEntry::new(db, die_entry));
                        recurse = false;
                    }
                }
                gimli::DW_TAG_variable => {
                    // this is public/global variable

                    let linkage_name_bytes =
                        match get_string_attr_raw(die, gimli::DW_AT_linkage_name, &unit_ref) {
                            Ok(Some(name)) => name,
                            Ok(None) => {
                                // there are many cases where this is None
                                // for example, local variables, function params
                                continue;
                            }
                            Err(e) => {
                                db.report_critical(format!(
                                    "Failed to get linkage name attribute: {e}"
                                ));
                                continue;
                            }
                        };

                    // find the name in the symbols map
                    if let Some((address, name)) = symbols.remove(&*linkage_name_bytes) {
                        let die_entry = DieEntryId::new(db, file_id, cu_offset, die.offset());
                        tracing::debug!(
                            "got function info for {}",
                            name.as_path(db),
                            // debug_print_die_entry(&die)
                        );
                        symbol_name_to_die
                            .insert(name, SymbolIndexEntry::new(db, address, die_entry));
                        recurse = false;
                    }
                }
                _ => {
                    // ignore
                }
            }
        }
    }

    // print out any symbols that we didn't match
    for (_, (address, demangled)) in symbols {
        tracing::debug!(
            "unmatched symbol: {} at {address:#x}",
            demangled.as_path(db)
        );
    }

    FileIndexEntries {
        function_name_to_die,
        symbol_name_to_die,
        address_range_to_function,
        cu_to_base_addr: cu_to_address,
    }
}
