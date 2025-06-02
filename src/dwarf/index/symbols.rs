//! Symbol indexing for functions and variables

use std::collections::{BTreeMap, btree_map::Entry};

use crate::database::Db;
use crate::dwarf::{
    navigation::get_roots,
    utils::{get_string_attr_raw, pretty_print_die_entry, to_range},
    {CompilationUnitId, Die},
};
use crate::file::File;
use crate::types::{FunctionIndexEntry, NameId, SymbolIndexEntry};

#[derive(Default)]
pub struct FileIndexEntries<'db> {
    /// map of function name to debugging entry
    pub function_name_to_die: BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>,
    /// map of symbol name to debugging entry
    pub symbol_name_to_die: BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>,
    /// list of function names with their address ranges
    pub address_range_to_function: Vec<(u64, u64, NameId<'db>)>,
}

pub fn index_symbols<'db>(db: &'db dyn Db, file: File) -> FileIndexEntries<'db> {
    tracing::debug!("indexing symbols for {}", file.path(db));
    let mut function_name_to_die = BTreeMap::new();
    let mut symbol_name_to_die = BTreeMap::new();
    let mut address_range_to_function = Vec::new();

    let roots = get_roots(db, file);

    for (cu_offset, unit_ref) in roots {
        let cu = CompilationUnitId::new(db, file, cu_offset);

        let Some(mut tree) = unit_ref.entries_tree(None).ok() else {
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

        let mut entries = unit_ref.entries();
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
                                    file.path(db)
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
                    match unit_ref
                        .die_ranges(die)
                        .map_err(anyhow::Error::from)
                        .and_then(to_range)
                    {
                        Ok(Some((start, end))) => {
                            address_range_to_function.push((start, end, name));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            db.report_critical(format!("Failed to get ranges: {e}"));
                            continue;
                        }
                    };

                    let die_entry = Die::new(db, file, cu_offset, die.offset());
                    tracing::debug!("got function info for {}", name.as_path(db),);
                    function_name_to_die.insert(name, FunctionIndexEntry::new(db, die_entry));
                    recurse = false;
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
                        let die_entry = Die::new(db, file, cu_offset, die.offset());
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
