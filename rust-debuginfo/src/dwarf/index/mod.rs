//! DWARF indexing functionality for fast lookups

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::Die;
use super::loader::{Offset, RawDie};
use super::unit::UnitRef;
use super::utils::{
    file_entry_to_path, get_lang_attr, get_string_attr, pretty_print_die_entry, to_range,
};
use super::visitor::{DieVisitor, DieWalker, walk_file};
use crate::address_tree::{AddressTree, FunctionAddressInfo};
use crate::database::Db;
use crate::dwarf::resolution::{FunctionDeclarationType, get_declaration_type};
use crate::file::{DebugFile, SourceFile};
use crate::types::NameId;

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
    function_declarations: BTreeMap<Offset, NameId<'db>>,
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
    /// Sometimes we'll find the same definition mulitple times
    /// in the same file due to compilation units
    ///
    /// For now, we'll just store the alternate locations
    /// although we'll probably need to do something else
    pub alternate_locations: Vec<Die<'db>>,
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
    function_addresses: Vec<(u64, u64, NameId<'db>)>,
    data: FileIndexData<'db>,
}

impl<'db> DieVisitor<'db> for FileIndexBuilder<'db> {
    fn visit_cu<'a>(walker: &mut DieWalker<'a, 'db, Self>, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
        if is_rust_cu(walker.db, &die, &unit_ref) {
            // get all referenced files
            let files = unit_ref
                .line_program
                .as_ref()
                .map(|lp| {
                    lp.header()
                        .file_names()
                        .iter()
                        .flat_map(|f| {
                            file_entry_to_path(f, &unit_ref)
                                .map(|path| SourceFile::new(walker.db, path))
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            walker.visitor.data.sources.extend(files);

            tracing::trace!(
                "walking cu: {:#010x}",
                unit_ref.header.offset().as_debug_info_offset().unwrap().0
            );
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
        walker.visitor.current_path.pop();
    }

    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        // also need to push the struct name to the current path
        // for name resolution
        let struct_name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .unwrap()
            .unwrap();
        walker.visitor.current_path.push(struct_name.clone());
        walker.walk_struct();
        walker.visitor.current_path.pop();
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let function_declaration_type = get_declaration_type(walker.db, &entry, &unit_ref);
        match function_declaration_type {
            FunctionDeclarationType::Closure => {
                // skip for now
                return;
            }
            FunctionDeclarationType::ClassMethodDeclaration
            | FunctionDeclarationType::Function { .. } => {
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
                    Ok(Some((start, end))) => {
                        walker
                            .visitor
                            .function_addresses
                            .push((start, end, name.clone()));
                        Some((start, end))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        walker
                            .db
                            .report_critical(format!("Failed to get ranges: {e}"));
                        None
                    }
                };

                let die = walker.get_die(entry.clone());
                match walker.visitor.data.functions.entry(name) {
                    Entry::Vacant(vacant_entry) => {
                        vacant_entry.insert(FunctionIndexEntry {
                            declaration_die: die,
                            relative_address_range,
                            linkage_name,
                            specification_die: None,
                            alternate_locations: vec![],
                        });
                    }
                    Entry::Occupied(occupied_entry) => {
                        occupied_entry.into_mut().alternate_locations.push(die);
                    }
                }
                walker
                    .visitor
                    .data
                    .function_declarations
                    .insert(entry.offset(), name.clone());

                // _ZN42_$LT$$RF$T$u20$as$u20$core..fmt..Debug$GT$3fmt17h17559c8d3719ef99E
                // _ZN44_$LT$$RF$T$u20$as$u20$core..fmt..Display$GT$3fmt17hfd46ad2ce46e94c4E
            }
            FunctionDeclarationType::ClassMethodImplementation(declaration_offset) => {
                let name = walker
                    .visitor
                    .data
                    .function_declarations
                    .get(&declaration_offset)
                    .cloned();
                if let Some(name) = name {
                    let spec_die = walker.get_die(entry.clone());
                    if let Some(declaration) = walker.visitor.data.functions.get_mut(&name) {
                        // this is an implementation of a class method
                        // we can update the existing entry with the implementation DIE
                        declaration.specification_die = Some(spec_die);

                        // find the name in the functions map
                        if declaration.relative_address_range.is_none() {
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
                            if let Some((start, end)) = relative_address_range {
                                walker
                                    .visitor
                                    .function_addresses
                                    .push((start, end, name.clone()));
                                // update the relative address range
                                declaration.relative_address_range.replace((start, end));
                            } else {
                                tracing::debug!(
                                    "No address range found for function: {}",
                                    pretty_print_die_entry(&entry, &unit_ref)
                                );
                            }
                        }
                    } else {
                        tracing::debug!(
                            "No function declaration found for offset: {declaration_offset:?}"
                        );
                    }
                } else {
                    tracing::debug!(
                        "No function declaration found for offset: {declaration_offset:?}"
                    );
                }
            }
            FunctionDeclarationType::InlinedFunctionImplementation(offset) => {
                // not handling for now
                tracing::trace!(
                    "Skipping inlined function implementation: at offset {:#010x}",
                    offset.0
                );
            }
        };
    }
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn build_file_index<'db>(db: &'db dyn Db, debug_file: DebugFile) -> FileIndex<'db> {
    let file = debug_file.file(db);

    let mut builder = FileIndexBuilder::default();
    tracing::info!("Indexing file: {}", file.path(db));
    walk_file(db, file, &mut builder);

    let FileIndexBuilder {
        function_addresses,
        mut data,
        ..
    } = builder;

    let function_addresses = function_addresses
        .into_iter()
        .map(|(start, end, name)| FunctionAddressInfo {
            start,
            end,
            name,
            // super redundant -- maybe we can remove somehow?
            file: debug_file,
        })
        .collect();
    data.function_addresses = AddressTree::new(function_addresses);

    tracing::trace!(
        "Indexed file data: {data:#?} for file: {}",
        debug_file.file(db).path(db)
    );

    super::FileIndex::new(db, data)
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
