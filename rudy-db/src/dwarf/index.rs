//! DWARF indexing functionality for fast lookups

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use gimli::Reader;
use itertools::Itertools;

use super::Die;
use super::loader::RawDie;
use super::unit::UnitRef;
use super::utils::{
    file_entry_to_path, get_lang_attr, get_string_attr, pretty_print_die_entry, to_range,
};
use super::visitor::{DieVisitor, DieWalker, walk_file};
use crate::Symbol;
use crate::address_tree::{AddressTree, FunctionAddressInfo};
use crate::database::Db;
use crate::dwarf::RawSymbol;
use crate::dwarf::{SymbolName, TypeName};
use crate::file::{DebugFile, SourceFile};

#[salsa::tracked(debug)]
pub struct FunctionIndexEntry<'db> {
    #[returns(ref)]
    pub data: FunctionData<'db>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, salsa::Update)]
pub struct FunctionData<'db> {
    /// Die entry for the function
    pub declaration_die: Die<'db>,
    /// Address range of the function relative to the binary
    pub address_range: Option<(u64, u64)>,
    pub name: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct TypeIndexEntry<'db> {
    pub die: Die<'db>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct Module<'db> {
    modules: BTreeMap<String, Module<'db>>,
    entries: Vec<Die<'db>>,
}

#[salsa::tracked(debug)]
pub struct ModuleIndex<'db> {
    #[returns(ref)]
    pub by_offset: Vec<ModuleRange>,
    #[returns(ref)]
    pub by_name: BTreeMap<String, Module<'db>>,
}

/// Namespace range representing a module's DIE offset coverage
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct ModuleRange {
    module_path: Vec<String>,
    start_offset: usize,
    end_offset: usize,
}

/// Visitor for building namespace ranges efficiently
/// Only visits namespaces and uses depth-first traversal to capture ranges
#[derive(Default)]
struct ModuleRangeBulider<'db> {
    module_ranges: Vec<ModuleRange>,
    modules: BTreeMap<String, Module<'db>>,
    current_path: Vec<String>,
    namespace_stack: Vec<(Vec<String>, usize)>, // (path, start_offset)
}

impl<'db> DieVisitor<'db> for ModuleRangeBulider<'db> {
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

        let start_offset = entry.offset().0;
        let die = walker.get_die(entry);

        let mut module_entry = &mut walker.visitor.modules;
        // Traverse or create the module path
        for segment in &walker.visitor.current_path {
            let module = module_entry.entry(segment.clone()).or_default();
            module_entry = &mut module.modules;
        }

        module_entry
            .entry(module_name.clone())
            .or_default()
            .entries
            .push(die);
        walker.visitor.current_path.push(module_name);

        // Record the start of this namespace
        walker
            .visitor
            .namespace_stack
            .push((walker.visitor.current_path.clone(), start_offset));

        walker.walk_namespace();

        // When we're done walking this namespace, record its range
        if let Some((path, start)) = walker.visitor.namespace_stack.pop() {
            let end_offset = walker.peek_next_offset().unwrap_or(usize::MAX);

            // Use the current offset as the end (we'll update this with the next entry)
            walker.visitor.module_ranges.push(ModuleRange {
                module_path: path,
                start_offset: start,
                end_offset,
            });
        }

        walker.visitor.current_path.pop();
    }
}

/// Build namespace ranges for efficient offset-based lookups
/// This is used to efficiently resolve type contexts without full indexing
#[salsa::tracked(returns(ref))]
pub fn module_index<'db>(db: &'db dyn Db, debug_file: DebugFile) -> ModuleIndex<'db> {
    let mut builder = ModuleRangeBulider::default();
    walk_file(db, debug_file, &mut builder);

    // Sort ranges by start offset and fix overlapping ranges
    let mut ranges = builder.module_ranges;
    ranges.sort_by_key(|r| r.start_offset);

    ModuleIndex::new(db, ranges, builder.modules)
}

/// Find the namespace path for a given DIE offset using range lookup
fn find_namespace_for_offset(ranges: &[ModuleRange], target_offset: usize) -> Vec<String> {
    // Find the most specific (deepest) namespace that contains this offset

    // first, find the first node that starts _after_ the target offset -- we'll search backwards
    // from this one
    let Ok(pos) = ranges.binary_search_by(|range| {
        if target_offset < range.start_offset {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        }
    }) else {
        tracing::warn!("No namespace found for offset {target_offset:#x}");
        return Vec::new();
    };

    let Some(path) = ranges[..pos]
        .iter()
        .rev()
        .find(|range| target_offset >= range.start_offset && target_offset < range.end_offset)
    else {
        tracing::warn!("No namespace found for offset {target_offset:#x}");
        return Vec::new();
    };

    path.module_path.clone()
}

/// Get typename by lazily resolving namespace context (avoids full indexing)
/// This is salsa-cached for performance
#[salsa::tracked(returns(ref))]
pub fn get_die_typename<'db>(db: &'db dyn Db, die: Die<'db>) -> Option<TypeName> {
    die.with_entry_and_unit(db, |target_entry, unit_ref| {
        // Get the name of the target DIE
        let name = get_string_attr(target_entry, gimli::DW_AT_name, unit_ref)
            .ok()
            .flatten()?;

        // Get the namespace ranges for this debug file (cached)
        let module_index = module_index(db, die.file(db));

        // Find the module path for this DIE using range lookup
        let module_path =
            find_namespace_for_offset(module_index.by_offset(db), target_entry.offset().0);

        tracing::debug!(
            "Found module path: {:?} for DIE: {} at offset {:#x}",
            module_path,
            die.print(db),
            target_entry.offset().0,
        );

        // Parse the typename with the module path
        TypeName::parse(&module_path, &name).ok()
    })
    .ok()
    .flatten()
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn index_debug_file_sources<'db>(
    db: &'db dyn Db,
    debug_file: DebugFile,
) -> (BTreeSet<PathBuf>, BTreeSet<SourceFile<'db>>) {
    let mut compile_dirs = BTreeSet::new();
    let mut sources = BTreeSet::new();

    let roots = super::navigation::get_roots(db, debug_file);
    for (_unit_offset, unit_ref) in &roots {
        let mut entries = unit_ref.entries();
        let Some((_, root)) = entries.next_dfs().ok().flatten() else {
            continue;
        };
        if is_rust_cu(db, root, unit_ref) {
            // get the compile directory
            if let Some(compile_dir) = &unit_ref.comp_dir {
                match compile_dir.to_string() {
                    Ok(compile_dir) => {
                        // if the compile directory is empty, we can skip it
                        if compile_dir.is_empty() {
                            tracing::debug!(
                                "Skipping empty compile directory for unit: {}",
                                pretty_print_die_entry(root, unit_ref)
                            );
                        } else {
                            compile_dirs.insert(PathBuf::from(compile_dir.to_string()));
                        }
                    }
                    Err(e) => {
                        db.report_error(format!(
                            "Failed to convert compile directory to string: {e}"
                        ));
                    }
                }
            }

            // get all referenced files
            let files = unit_ref
                .line_program
                .as_ref()
                .map(|lp| {
                    lp.header()
                        .file_names()
                        .iter()
                        .flat_map(|f| {
                            file_entry_to_path(db, f, unit_ref)
                                .map(|path| SourceFile::new(db, path))
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            sources.extend(files);
        }
    }

    (compile_dirs, sources)
}

fn is_rust_cu(db: &dyn Db, root: &RawDie<'_>, unit_ref: &UnitRef<'_>) -> bool {
    match get_lang_attr(root, unit_ref) {
        Ok(Some(lang)) if lang == gimli::DW_LANG_Rust => {
            // this is a Rust file, we can index it
            true
        }
        Ok(_) => {
            // not a rust file / language not found
            tracing::debug!(
                "skipping non-Rust compilation unit: {}",
                pretty_print_die_entry(root, unit_ref)
            );
            false
        }
        Err(e) => {
            db.report_error(format!(
                "could not get language of compilation unit: {e}: \n{}",
                pretty_print_die_entry(root, unit_ref)
            ));
            false
        }
    }
}

// ===== TARGETED INDEXING FUNCTIONS =====

/// Targeted function index containing only functions
#[salsa::tracked(debug)]
pub struct FunctionIndex<'db> {
    #[returns(ref)]
    pub by_symbol_name: BTreeMap<SymbolName, FunctionIndexEntry<'db>>,
    #[returns(ref)]
    pub by_relative_address: AddressTree,
    #[returns(ref)]
    pub by_absolute_address: AddressTree,
}

/// Visitor for building function index efficiently
struct FunctionIndexBuilder<'db> {
    functions: BTreeMap<SymbolName, FunctionIndexEntry<'db>>,
    absolute_function_addresses: Vec<FunctionAddressInfo>,
    relative_function_addresses: Vec<FunctionAddressInfo>,
    symbol_map: &'db BTreeMap<RawSymbol, Symbol>,
}

impl<'db> FunctionIndexBuilder<'db> {
    /// Create a new function index builder
    pub fn new(symbol_map: &'db BTreeMap<RawSymbol, Symbol>) -> Self {
        Self {
            functions: BTreeMap::new(),
            absolute_function_addresses: Vec::new(),
            relative_function_addresses: Vec::new(),
            symbol_map,
        }
    }
}

impl<'db> DieVisitor<'db> for FunctionIndexBuilder<'db> {
    fn visit_cu<'a>(walker: &mut DieWalker<'a, 'db, Self>, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
        if is_rust_cu(walker.db, &die, &unit_ref) {
            walker.walk_cu();
        }
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let mut linkage_name = match get_string_attr(&entry, gimli::DW_AT_linkage_name, &unit_ref) {
            Ok(Some(linkage_name)) => linkage_name,
            Ok(None) => {
                tracing::trace!(
                    "Skipping function with no linkage name: {}",
                    pretty_print_die_entry(&entry, &unit_ref)
                );
                return;
            }
            Err(e) => {
                tracing::error!(
                    "Failed to get linkage name for function: {e}: \n{}",
                    pretty_print_die_entry(&entry, &unit_ref)
                );
                return;
            }
        };

        if walker.file.relocatable(walker.db) {
            linkage_name.insert(0, '_'); // Ensure linkage name starts with underscore for relocatable files
        }

        // find if the symbol is actually linked in the binary
        if let Some(symbol) = walker
            .visitor
            .symbol_map
            .get(&RawSymbol::new(linkage_name.as_bytes().to_vec()))
        {
            let address_range = unit_ref
                .die_ranges(&entry)
                .map_err(anyhow::Error::from)
                .and_then(to_range)
                .unwrap_or(None);

            let die = walker.get_die(entry);

            let function_data = FunctionData {
                declaration_die: die,
                address_range,
                name: symbol.name.lookup_name.clone(),
                specification_die: None,
                alternate_locations: vec![],
            };

            // Add to address tree if we have address range
            if let Some((relative_start, relative_end)) = address_range {
                walker
                    .visitor
                    .absolute_function_addresses
                    .push(FunctionAddressInfo {
                        start: symbol.address,
                        end: symbol.address + relative_end - relative_start,
                        relative_start,
                        name: symbol.name.clone(),
                        file: walker.file,
                    });
                walker
                    .visitor
                    .relative_function_addresses
                    .push(FunctionAddressInfo {
                        start: relative_start,
                        end: relative_end,
                        relative_start,
                        name: symbol.name.clone(),
                        file: walker.file,
                    });
            } else {
                tracing::trace!(
                    "Function {} has no address range in debug file {}",
                    symbol.name,
                    walker.file.name(walker.db)
                );
            }

            let entry = FunctionIndexEntry::new(walker.db, function_data);
            walker.visitor.functions.insert(symbol.name.clone(), entry);
        } else {
            tracing::trace!(
                "Skipping unlinked function: {linkage_name} in {:#?}",
                walker
                    .visitor
                    .symbol_map
                    .values()
                    .map(|s| &s.name)
                    .join("\n")
            );
        }
    }
}

/// Index only functions in debug file using visitor pattern
#[salsa::tracked(returns(ref))]
pub fn function_index<'db>(
    db: &'db dyn Db,
    debug_file: DebugFile,
    symbol_map: &'db BTreeMap<RawSymbol, Symbol>,
) -> FunctionIndex<'db> {
    let mut builder = FunctionIndexBuilder::new(symbol_map);
    walk_file(db, debug_file, &mut builder);

    FunctionIndex::new(
        db,
        builder.functions,
        AddressTree::new(builder.relative_function_addresses),
        AddressTree::new(builder.absolute_function_addresses),
    )
}

pub fn find_type_by_name<'db>(
    db: &'db dyn Db,
    debug_file: DebugFile,
    type_name: TypeName,
) -> Option<Die<'db>> {
    let module_index = module_index(db, debug_file);

    // let indexed = super::navigation::function_index(db, debug_file);
    // let type_name = TypeName::parse(type_name).ok()?;

    // Search through all debug files to find the type
    let mut modules = module_index.by_name(db);
    let mut found_module = vec![];

    // tracing::info!("")

    for segment in &type_name.module.segments {
        if let Some(module) = modules.get(segment) {
            found_module = module.entries.clone();
            modules = &module.modules;
            tracing::info!(
                "Found module segment {segment} in debug file {} {:#?}",
                debug_file.name(db),
                modules.keys().collect::<Vec<_>>()
            );
        } else {
            tracing::info!(
                "Module segment {segment} not found in debug file {}",
                debug_file.name(db)
            );
        }
    }

    if found_module.is_empty() {
        tracing::warn!(
            "No module found for type {type_name:#?} in debug file {}\n\n{:#?}",
            debug_file.name(db),
            module_index.by_name(db).keys().collect::<Vec<_>>(),
        );
        return None;
    }

    tracing::info!(
        "Searching for type {type_name:#?} in modules: {:?}",
        found_module
    );

    // Now search for the type in the remaining modules
    for module in found_module {
        tracing::info!(
            "Searching in module: {} at location: {}",
            module.name(db).unwrap(),
            module.location(db)
        );
        // find the type name in the module
        for entry in module.children(db).unwrap_or_default() {
            if let Ok(name) = entry.name(db) {
                let Ok(parsed) = TypeName::parse(&type_name.module.segments, &name) else {
                    tracing::warn!("Failed to parse type name `{name}` in module {module:?}");
                    continue;
                };
                tracing::info!("Checking type name: {name} vs {}", type_name.name);
                if parsed.typedef.matching_type(&type_name.typedef) {
                    tracing::info!("Found type {type_name:#?}  {}", entry.location(db));
                    return Some(entry);
                }
            }
        }
    }

    None
}
