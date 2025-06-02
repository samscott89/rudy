//! DWARF indexing functionality for fast lookups

use std::collections::{BTreeMap, BTreeSet};

use super::loader::RawDie;
use super::unit::UnitRef;
use super::utils::{get_lang_attr, get_string_attr, get_string_attr_raw, to_range};
use super::visitor::{DieVisitor, DieWalker, walk_file};
use super::{CompilationUnitId, Die};
use crate::address_tree::AddressTree;
use crate::database::Db;
use crate::dwarf::navigation::get_roots;
use crate::dwarf::utils::pretty_print_die_entry;
use crate::file::{DebugFile, File, SourceFile};
use crate::types::NameId;

// mod symbols;
// mod types;

// pub use symbols::index_symbols;
// pub use types::index_types;

/// Pre-computed index for fast lookups
#[salsa::tracked(debug)]
pub struct FileIndex<'db> {
    #[returns(ref)]
    pub data: FileIndexData<'db>,
}

/// Index data structure containing all mappings
#[derive(Default, Hash, PartialEq, Debug)]
pub struct FileIndexData<'db> {
    pub modules: BTreeMap<NameId<'db>, ModuleIndexEntry<'db>>,
    pub functions: BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>,
    pub symbols: BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>,
    pub types: BTreeMap<NameId<'db>, TypeIndexEntry<'db>>,
    pub sources: BTreeSet<SourceFile<'db>>,
    pub function_addresses: AddressTree<'db>,
}

#[salsa::tracked(debug)]
pub struct ModuleIndexEntry<'db> {
    /// DIE entry for the module
    pub die: Die<'db>,
}

#[derive(Clone, Hash, PartialEq, Debug)]
pub struct FunctionIndexEntry<'db> {
    /// Die entry for the function
    pub declaration_die: Die<'db>,
    /// Address range of the function relative to the base address of the compilation unit
    pub relative_address_range: Option<(u64, u64)>,
    pub linkage_name: Option<String>,
    pub specification_die: Option<Die<'db>>,
}

#[salsa::tracked(debug)]
pub struct SymbolIndexEntry<'db> {
    pub die: Die<'db>,
}

#[salsa::tracked(debug)]
pub struct TypeIndexEntry<'db> {
    pub die: Die<'db>,
}

unsafe impl salsa::Update for FileIndexData<'_> {
    unsafe fn maybe_update(_: *mut Self, _: Self) -> bool {
        // IndexData should never change after creation
        todo!()
    }
}

#[derive(Default)]
struct FileIndexBuilder<'db> {
    current_path: Vec<String>,
    data: FileIndexData<'db>,
}

impl<'db> DieVisitor<'db> for FileIndexBuilder<'db> {
    fn visit_cu<'a>(walker: &mut DieWalker<'a, 'db, Self>, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
        if is_rust_cu(walker.db, &die, &unit_ref) {
            walker.walk_cu();
        }
    }

    fn visit_namespace<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let module_name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .unwrap()
            .unwrap();
        walker.visitor.data.modules.insert(
            NameId::new(
                walker.db,
                walker.visitor.current_path.clone(),
                module_name.clone(),
            ),
            ModuleIndexEntry::new(walker.db, walker.get_die(entry)),
        );
        walker.visitor.current_path.push(module_name);
        walker.walk_namespace();
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let Some(function_name) = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .ok()
            .flatten()
        else {
            tracing::debug!(
                "No function name found for entry: {}",
                pretty_print_die_entry(&entry, &unit_ref)
            );
            return;
        };
        let linkage_name = get_string_attr(&entry, gimli::DW_AT_linkage_name, &unit_ref)
            .ok()
            .flatten();

        let name = NameId::new(
            walker.db,
            walker.visitor.current_path.clone(),
            function_name.clone(),
        );

        // find the name in the functions map
        let relative_address_range = match unit_ref
            .die_ranges(&entry)
            .map_err(anyhow::Error::from)
            .and_then(to_range)
        {
            Ok(Some((start, end))) => Some((start, end)),
            Ok(None) => None,
            Err(e) => {
                walker
                    .db
                    .report_critical(format!("Failed to get ranges: {e}"));
                None
            }
        };

        walker.visitor.data.functions.insert(
            name,
            FunctionIndexEntry {
                declaration_die: walker.get_die(entry),
                relative_address_range,
                linkage_name: None,
                specification_die: None,
            },
        );

        // // skip if it references some other type?
        // if die
        //     .attr_value(gimli::DW_AT_specification)
        //     .is_ok_and(|v| v.is_some())
        // {
        //     continue;
        // }

        // if die
        //     .attr_value(gimli::DW_AT_abstract_origin)
        //     .is_ok_and(|v| v.is_some())
        // {
        //     continue;
        // }

        // if die
        //     .attr_value(gimli::DW_AT_external)
        //     .is_ok_and(|v| v.is_some())
        // {
        //     // external symbol -- skip
        //     continue;
        // }
        // if die
        //     .attr_value(gimli::DW_AT_prototyped)
        //     .is_ok_and(|v| v.is_some())
        // {
        //     // external symbol -- skip
        //     continue;
        // }

        // // TODO: future cases to handle:
        // // 1. DW_AT_declaration = Flag(true)
        // //    - we have an _incomplete_ declaration of a function
        // //      and need to find what completes it
        // // 2. DW_AT_specification = Flag(true)
        // //    - we have a function that is defined in another
        // //      compilation unit, and we need to find that unit
        // // 3. DW_AT_inline = Inline(DW_INL_inlined)
        // //    - An inlined function
        // let linkage_name_bytes =
        //     match get_string_attr_raw(die, gimli::DW_AT_linkage_name, &unit_ref) {
        //         Ok(Some(name)) => name,
        //         Ok(None) => {
        //             let entry = pretty_print_die_entry(die, &unit_ref);
        //             tracing::error!("No linkage name attribute: \n{entry} in {}", file.path(db));
        //             db.report_critical(format!("no linkage name attribute?"));
        //             continue;
        //         }
        //         Err(e) => {
        //             db.report_critical(format!("Failed to get linkage name attribute: {e}"));
        //             continue;
        //         }
        //     };

        // // find the name in the functions map
        // match unit_ref
        //     .die_ranges(die)
        //     .map_err(anyhow::Error::from)
        //     .and_then(to_range)
        // {
        //     Ok(Some((start, end))) => {
        //         address_range_to_function.push((start, end, name));
        //     }
        //     Ok(None) => {}
        //     Err(e) => {
        //         db.report_critical(format!("Failed to get ranges: {e}"));
        //         continue;
        //     }
        // };

        // let die_entry = Die::new(db, file, cu_offset, die.offset());
        // tracing::debug!("got function info for {}", name.as_path(db),);
        // function_name_to_die.insert(name, FunctionIndexEntry::new(db, die_entry));
        // recurse = false;
    }
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn build_file_index<'db>(db: &'db dyn Db, debug_file: DebugFile) -> FileIndex<'db> {
    let file = debug_file.file(db);

    let mut builder = FileIndexBuilder::default();
    walk_file(db, file, &mut builder);

    // for (cu_offset, unit_ref) in &roots {
    // }

    // let mut current_path = vec![];
    // let mut path_depths = vec![];
    // let mut current_offset = 0;

    // // at times we'll know that there's no need to recurse further into the tree
    // // so we can skip the `next_dfs` call and just continue with the next sibling
    // let mut continue_recursing = true;

    // for (cu_offset, unit_ref) in roots {
    //     let mut tree = match { unit_ref.entries_tree(None) } {
    //         Ok(tree) => tree,
    //         Err(e) => {
    //             db.report_error(format!("Failed to get entries tree: {e}"));
    //             continue;
    //         }
    //     };
    //     let root = match tree.root() {
    //         Ok(root) => root,
    //         Err(e) => {
    //             db.report_error(format!("Failed to get root entry: {e}"));
    //             continue;
    //         }
    //     };
    //     let root = root.entry();
    //     if !is_rust_cu(db, root, &unit_ref) {
    //         continue;
    //     }

    //     let mut entries = unit_ref.entries();
    //     loop {
    //         let die = if continue_recursing {
    //             match entries.next_dfs() {
    //                 Ok(Some((offset, die))) => {
    //                     current_offset += offset;
    //                     die
    //                 }
    //                 Ok(None) => break,
    //                 Err(e) => {
    //                     db.report_critical(format!("Failed to get entry: {e}"));
    //                     continue;
    //                 }
    //             }
    //         } else {
    //             // continue recursing
    //             continue_recursing = true;
    //             match entries.next_sibling() {
    //                 Ok(Some(die)) => {
    //                     // current offset stays the same
    //                     die
    //                 }
    //                 Ok(None) => {
    //                     // we've run out of siblings, but we might still have
    //                     // another parent (which `next_dfs` will check)
    //                     continue;
    //                 }
    //                 Err(e) => {
    //                     db.report_critical(format!("Failed to get entry: {e}"));
    //                     continue;
    //                 }
    //             }
    //         };

    //         // check if we need to pop one or many paths from the stack
    //         loop {
    //             match path_depths.last() {
    //                 None => {
    //                     // we're at the root
    //                     break;
    //                 }
    //                 Some(d) if *d < current_offset => {
    //                     // we're at a child of the top namespace visited
    //                     break;
    //                 }
    //                 _ => {
    //                     // we're at a sibling or parent of the current offset
    //                     // pop the last path
    //                     current_path.pop();
    //                     path_depths.pop();
    //                 }
    //             }
    //         }

    //         match die.tag() {
    //             gimli::DW_TAG_namespace => {
    //                 let name = match get_string_attr(die, gimli::DW_AT_name, &unit_ref) {
    //                     Ok(Some(name)) => name,
    //                     Ok(None) => {
    //                         db.report_critical(format!("Failed to get namespace name"));
    //                         continue;
    //                     }
    //                     Err(e) => {
    //                         db.report_critical(format!("Failed to get namespace name: {e}"));
    //                         continue;
    //                     }
    //                 };
    //                 tracing::debug!(
    //                     ?current_path,
    //                     ?path_depths,
    //                     "got namespace: {name} at offset {current_offset}"
    //                 );
    //                 current_path.push(name);
    //                 path_depths.push(current_offset);
    //             }

    //             gimli::DW_TAG_pointer_type | gimli::DW_TAG_array_type => {
    //                 // these are composite types that we don't want to
    //                 // index by name (we'll handle these dynamically on lookup)
    //                 continue_recursing = false;
    //             }

    //             gimli::DW_TAG_base_type
    //             | gimli::DW_TAG_structure_type
    //             | gimli::DW_TAG_union_type
    //             | gimli::DW_TAG_enumeration_type
    //             | gimli::DW_TAG_atomic_type
    //             | gimli::DW_TAG_const_type => {
    //                 let name = match get_string_attr(die, gimli::DW_AT_name, &unit_ref) {
    //                     Ok(Some(name)) => name,
    //                     Ok(None) => {
    //                         db.report_critical(format!(
    //                             "No type name found for entry: {}",
    //                             debug_print_die_entry(die)
    //                         ));
    //                         continue;
    //                     }
    //                     Err(e) => {
    //                         db.report_critical(format!("Failed to get type name: {e}"));
    //                         continue;
    //                     }
    //                 };
    //                 tracing::debug!(?current_path, tag=%die.tag(), "found type: {name}");
    //                 let name = NameId::new(db, current_path.clone(), name.clone());
    //                 let die_entry = Die::new(db, file, cu_offset, die.offset());
    //                 let existing = name_to_die.insert(name, TypeIndexEntry::new(db, die_entry));
    //                 if let Some(existing) = existing {
    //                     tracing::debug!(
    //                         "Duplicate type name: {} at offset {die_entry:?} and {}",
    //                         name.as_path(db),
    //                         existing.die(db).print(db)
    //                     );
    //                 }
    //                 let existing = die_to_name.insert(die_entry, name);
    //                 if let Some(existing) = existing {
    //                     debug_assert!(false, "duplicate die entry for {}", existing.name(db))
    //                 }

    //                 // we don't want to recurse into the children of this type
    //                 continue_recursing = false;
    //             }
    //             _ => {
    //                 // ignore
    //             }
    //         }
    //     }
    // }

    // let mut names_by_file: BTreeMap<File, BTreeMap<Vec<u8>, _>> = BTreeMap::new();

    // for (file_id, names) in names_by_file {
    //     let file_entries = super::index_symbols(db, file_id, names);

    //     function_name_to_die.extend(file_entries.function_name_to_die);
    //     symbol_name_to_die.extend(file_entries.symbol_name_to_die);
    //     address_range_to_function.extend(file_entries.address_range_to_function);
    //     cu_to_base_addr.extend(file_entries.cu_to_base_addr);

    //     let (name_to_die, die_to_name) = super::index_types(db, file_id);
    //     type_name_to_die.extend(name_to_die);
    //     die_to_type_name.extend(die_to_name);

    //     let roots = super::parse_roots(db, file_id);
    //     for root in roots {
    //         let cu = root.cu(db);
    //         if let Some(base_addr) = cu_to_base_addr.get(&root.cu(db)) {
    //             let (start, end) = root.address_range(db);
    //             address_range_to_cu.push((base_addr + start, base_addr + end, cu));
    //         }

    //         for file in root.files(db) {
    //             file_to_cu.entry(*file).or_default().push(cu)
    //         }
    //     }
    // }

    // // sort the lists
    // address_range_to_function.sort_unstable();
    // address_range_to_cu.sort_unstable();

    super::FileIndex::new(db, builder.data)
}

fn is_rust_cu(db: &dyn Db, root: &RawDie<'_>, unit_ref: &UnitRef<'_>) -> bool {
    match get_lang_attr(root, &unit_ref) {
        Ok(Some(lang)) if lang == gimli::DW_LANG_Rust => {
            // this is a Rust file, we can index it
            true
        }
        Ok(_) => {
            // not a rust file / language not found
            tracing::debug!(
                "skipping non-Rust compilation unit: {}",
                pretty_print_die_entry(root, &unit_ref)
            );
            false
        }
        Err(e) => {
            db.report_error(format!(
                "could not get language of compilation unit: {e}: \n{}",
                pretty_print_die_entry(root, &unit_ref)
            ));
            false
        }
    }
}
