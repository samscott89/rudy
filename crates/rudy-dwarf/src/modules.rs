use std::collections::BTreeMap;

use crate::{
    die::{cu::is_rust_cu, utils::get_string_attr, UnitRef},
    file::RawDie,
    visitor::{walk_file, DieVisitor, DieWalker},
    DebugFile, Die, DwarfDb,
};

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct Module<'db> {
    pub(crate) modules: BTreeMap<String, Module<'db>>,
    pub(crate) entries: Vec<Die<'db>>,
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
    pub(crate) module_path: Vec<String>,
    pub(crate) start_offset: usize,
    pub(crate) end_offset: usize,
}

/// Visitor for building namespace ranges efficiently
/// Only visits namespaces and uses depth-first traversal to capture ranges
#[derive(Default)]
struct ModuleRangeBuilder<'db> {
    module_ranges: Vec<ModuleRange>,
    modules: BTreeMap<String, Module<'db>>,
    current_path: Vec<String>,
    namespace_stack: Vec<(Vec<String>, usize)>, // (path, start_offset)
}

impl<'db> DieVisitor<'db> for ModuleRangeBuilder<'db> {
    fn visit_cu<'a>(walker: &mut DieWalker<'a, 'db, Self>, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
        if is_rust_cu(&die, &unit_ref) {
            walker.walk_cu();
        }
    }

    fn visit_die<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        die: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        if die.tag() == gimli::DW_TAG_namespace {
            // Only visit namespaces, skip other DIEs
            Self::visit_namespace(walker, die, unit_ref);
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
pub fn module_index<'db>(db: &'db dyn DwarfDb, debug_file: DebugFile) -> ModuleIndex<'db> {
    let mut builder = ModuleRangeBuilder::default();
    walk_file(db, debug_file, &mut builder);

    // Sort ranges by start offset and fix overlapping ranges
    let mut ranges = builder.module_ranges;
    ranges.sort_by_key(|r| r.start_offset);

    ModuleIndex::new(db, ranges, builder.modules)
}
