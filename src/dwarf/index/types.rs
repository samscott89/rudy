//! Type indexing for DWARF type definitions

use std::collections::BTreeMap;

use crate::database::Db;
use crate::dwarf::{
    Die,
    navigation::get_roots,
    utils::{debug_print_die_entry, get_string_attr, pretty_print_die_entry},
};
use crate::file::FileId;
use crate::types::{NameId, TypeIndexEntry};

pub fn index_types<'db>(
    db: &'db dyn Db,
    file_id: FileId<'db>,
) -> (
    // map of type name to die entry
    BTreeMap<NameId<'db>, TypeIndexEntry<'db>>,
    // map of offset (entry ID) to type name
    BTreeMap<Die<'db>, NameId<'db>>,
) {
    let Some(dwarf) = db.get_file(file_id).and_then(|f| f.dwarf()) else {
        return Default::default();
    };

    let mut name_to_die = BTreeMap::new();
    let mut die_to_name = BTreeMap::new();

    let roots = get_roots(db, file_id);

    let mut current_path = vec![];
    let mut path_depths = vec![];
    let mut current_offset = 0;
    let mut recurse = true;

    for (cu_offset, unit) in roots {
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
        loop {
            let die = if recurse {
                match entries.next_dfs() {
                    Ok(Some((offset, die))) => {
                        current_offset += offset;
                        die
                    }
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
                    Ok(Some(die)) => {
                        // current offset stays the same
                        die
                    }
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

            // check if we need to pop one or many paths from the stack
            loop {
                match path_depths.last() {
                    None => {
                        // we're at the root
                        break;
                    }
                    Some(d) if *d < current_offset => {
                        // we're at a child of the top namespace visited
                        break;
                    }
                    _ => {
                        // we're at a sibling or parent of the current offset
                        // pop the last path
                        current_path.pop();
                        path_depths.pop();
                    }
                }
            }

            match die.tag() {
                gimli::DW_TAG_namespace => {
                    let name = match get_string_attr(die, gimli::DW_AT_name, &unit_ref) {
                        Ok(Some(name)) => name,
                        Ok(None) => {
                            db.report_critical(format!("Failed to get namespace name"));
                            continue;
                        }
                        Err(e) => {
                            db.report_critical(format!("Failed to get namespace name: {e}"));
                            continue;
                        }
                    };
                    tracing::debug!(
                        ?current_path,
                        ?path_depths,
                        "got namespace: {name} at offset {current_offset}"
                    );
                    current_path.push(name);
                    path_depths.push(current_offset);
                }

                gimli::DW_TAG_pointer_type | gimli::DW_TAG_array_type => {
                    // these are composite types that we don't want to
                    // index by name (we'll handle these dynamically on lookup)
                    recurse = false;
                }

                gimli::DW_TAG_base_type
                | gimli::DW_TAG_structure_type
                | gimli::DW_TAG_union_type
                | gimli::DW_TAG_enumeration_type
                | gimli::DW_TAG_atomic_type
                | gimli::DW_TAG_const_type => {
                    let name = match get_string_attr(die, gimli::DW_AT_name, &unit_ref) {
                        Ok(Some(name)) => name,
                        Ok(None) => {
                            db.report_critical(format!(
                                "No type name found for entry: {}",
                                debug_print_die_entry(die)
                            ));
                            continue;
                        }
                        Err(e) => {
                            db.report_critical(format!("Failed to get type name: {e}"));
                            continue;
                        }
                    };
                    tracing::debug!(?current_path, tag=%die.tag(), "found type: {name}");
                    let name = NameId::new(db, current_path.clone(), name.clone());
                    let die_entry = Die::new(db, file_id, cu_offset, die.offset());
                    let existing = name_to_die.insert(name, TypeIndexEntry::new(db, die_entry));
                    if let Some(existing) = existing {
                        tracing::debug!(
                            "Duplicate type name: {} at offset {die_entry:?} and {existing:?}",
                            name.as_path(db)
                        );
                    }
                    let existing = die_to_name.insert(die_entry, name);
                    debug_assert!(existing.is_none(), "duplicate die entry: {existing:#?}");

                    // we don't want to recurse into the children of this type
                    recurse = false;
                }
                _ => {
                    // ignore
                }
            }
        }
    }

    (name_to_die, die_to_name)
}
