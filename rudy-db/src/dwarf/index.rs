//! DWARF indexing functionality for fast lookups

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use gimli::Reader;

use super::Die;
use super::loader::RawDie;
use super::unit::UnitRef;
use super::utils::{
    file_entry_to_path, get_lang_attr, get_string_attr, pretty_print_die_entry, to_range,
};
use super::visitor::{DieVisitor, DieWalker, walk_file};
use crate::address_tree::{AddressTree, FunctionAddressInfo};
use crate::database::Db;
use crate::dwarf::RawSymbol;
use crate::dwarf::{
    SymbolName, TypeName,
    resolution::{FunctionDeclarationType, get_declaration_type},
};
use crate::file::{DebugFile, SourceFile};

#[salsa::tracked(debug)]
pub struct ModuleIndexEntry<'db> {
    /// DIE entry for the module
    pub die: Die<'db>,
}

#[salsa::tracked(debug)]
pub struct FunctionIndexEntry<'db> {
    #[returns(ref)]
    pub data: FunctionData<'db>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, salsa::Update)]
pub struct FunctionData<'db> {
    /// Die entry for the function
    pub declaration_die: Die<'db>,
    /// Address range of the function relative to the base address of the compilation unit
    pub relative_address_range: Option<(u64, u64)>,
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

#[salsa::tracked(debug)]
pub struct TypeIndexEntry<'db> {
    pub die: Die<'db>,
}

/// Namespace range representing a module's DIE offset coverage
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamespaceRange {
    module_path: Vec<String>,
    start_offset: usize,
    end_offset: usize,
}

/// Visitor for building namespace ranges efficiently
/// Only visits namespaces and uses depth-first traversal to capture ranges
#[derive(Default)]
struct NamespaceRangeBuilder {
    namespace_ranges: Vec<NamespaceRange>,
    current_path: Vec<String>,
    namespace_stack: Vec<(Vec<String>, usize)>, // (path, start_offset)
}

impl<'db> DieVisitor<'db> for NamespaceRangeBuilder {
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
        walker.visitor.current_path.push(module_name);

        // Record the start of this namespace
        let start_offset = entry.offset().0;
        walker
            .visitor
            .namespace_stack
            .push((walker.visitor.current_path.clone(), start_offset));

        walker.walk_namespace();

        // When we're done walking this namespace, record its range
        if let Some((path, start)) = walker.visitor.namespace_stack.pop() {
            let end_offset = walker.peek_next_offset().unwrap_or(usize::MAX);

            // Use the current offset as the end (we'll update this with the next entry)
            walker.visitor.namespace_ranges.push(NamespaceRange {
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
pub fn build_namespace_ranges<'db>(db: &'db dyn Db, debug_file: DebugFile) -> Vec<NamespaceRange> {
    let mut builder = NamespaceRangeBuilder::default();
    walk_file(db, debug_file, &mut builder);

    // Sort ranges by start offset and fix overlapping ranges
    let mut ranges = builder.namespace_ranges;
    ranges.sort_by_key(|r| r.start_offset);
    ranges
}

/// Find the namespace path for a given DIE offset using range lookup
fn find_namespace_for_offset(ranges: &[NamespaceRange], target_offset: usize) -> Vec<String> {
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
        let namespace_ranges = build_namespace_ranges(db, die.file(db));

        // Find the module path for this DIE using range lookup
        let module_path = find_namespace_for_offset(namespace_ranges, target_entry.offset().0);

        tracing::debug!(
            "Found module path: {:?} for DIE: {} at offset {:#x} (ranges: {})",
            module_path,
            die.print(db),
            target_entry.offset().0,
            namespace_ranges.len()
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
    pub by_address: AddressTree,
}

/// Visitor for building function index efficiently
#[derive(Default)]
struct FunctionIndexBuilder<'db> {
    functions: BTreeMap<SymbolName, FunctionIndexEntry<'db>>,
    function_addresses: Vec<FunctionAddressInfo>,
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
        let function_declaration_type = get_declaration_type(walker.db, &entry, &unit_ref);

        if matches!(
            function_declaration_type,
            FunctionDeclarationType::Function { .. }
                | FunctionDeclarationType::ClassMethodDeclaration
        ) {
            if let (Ok(Some(function_name)), Ok(Some(linkage_name))) = (
                get_string_attr(&entry, gimli::DW_AT_name, &unit_ref),
                get_string_attr(&entry, gimli::DW_AT_linkage_name, &unit_ref),
            ) {
                if let Ok(demangled) = RawSymbol::new(linkage_name.as_bytes().to_vec()).demangle() {
                    let relative_address_range = unit_ref
                        .die_ranges(&entry)
                        .map_err(anyhow::Error::from)
                        .and_then(to_range)
                        .unwrap_or(None);

                    let die = walker.get_die(entry);

                    let function_data = FunctionData {
                        declaration_die: die,
                        relative_address_range,
                        name: function_name,
                        specification_die: None,
                        alternate_locations: vec![],
                    };

                    // Add to address tree if we have address range
                    if let Some((start, end)) = relative_address_range {
                        walker.visitor.function_addresses.push(FunctionAddressInfo {
                            start,
                            end,
                            name: demangled.clone(),
                            file: walker.file,
                        });
                    }

                    let entry = FunctionIndexEntry::new(walker.db, function_data);
                    walker.visitor.functions.insert(demangled, entry);
                }
            }
        }
    }
}

/// Index only functions in debug file using visitor pattern
#[salsa::tracked(returns(ref))]
pub fn function_index<'db>(db: &'db dyn Db, debug_file: DebugFile) -> FunctionIndex<'db> {
    // TODO: it would be good if we took in the symbol -> address map
    // for this debug file so that we can immediately shift the addresses
    // to absolute addresses instead of relative ones

    let mut builder = FunctionIndexBuilder::default();
    walk_file(db, debug_file, &mut builder);

    FunctionIndex::new(
        db,
        builder.functions,
        AddressTree::new(builder.function_addresses),
    )
}
