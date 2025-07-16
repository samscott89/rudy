use std::{collections::BTreeMap, fmt};

use crate::{
    die::{cu::is_rust_cu, utils::get_string_attr},
    visitor::{walk_file, DieVisitor, DieWalker, VisitorNode},
    DebugFile, Die, DwarfDb,
};

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct Module {
    pub(crate) modules: BTreeMap<String, Module>,
    pub entries: Vec<Die>,
}

#[salsa::tracked(debug)]
pub struct ModuleIndex<'db> {
    #[returns(ref)]
    pub by_offset: Vec<ModuleRange>,
    #[returns(ref)]
    pub by_name: Module,
}

impl<'db> ModuleIndex<'db> {
    /// Find the module at a specific offset
    pub fn find_by_offset(&self, db: &'db dyn DwarfDb, offset: usize) -> Option<&'db ModuleRange> {
        let ranges = self.by_offset(db);
        // Find the most specific (deepest) namespace that contains this offset
        let pos = ranges
            // find the first point at which the range starts _after_ the offset
            .partition_point(|range| range.start_offset <= offset);

        tracing::trace!("Partition point: {:?}", ranges.get(pos));

        // then, work backwards to find the first range that contains the offset
        ranges[..pos]
            .iter()
            .rev()
            .find(|range| offset >= range.start_offset && offset < range.end_offset)
    }

    /// Find a module by its name
    pub fn find_by_path(&self, db: &'db dyn DwarfDb, module_path: &[String]) -> Option<&Module> {
        let mut module = self.by_name(db);

        for segment in module_path {
            module = module.modules.get(segment)?;
        }

        Some(module)
    }
}

/// Namespace range representing a module's DIE offset coverage
#[derive(Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct ModuleRange {
    pub(crate) module_path: Vec<String>,
    pub(crate) die: Die,
    pub(crate) start_offset: usize,
    pub(crate) end_offset: usize,
}

impl fmt::Debug for ModuleRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            f.debug_struct("ModuleRange")
                .field("module_path", &self.module_path.join("::"))
                .field("die", &self.die.location(db))
                .field("start_offset", &format!("{:#010x}", self.start_offset))
                .field("end_offset", &format!("{:#010x}", self.end_offset))
                .finish()
        })
        .unwrap_or_else(|| {
            f.debug_struct("ModuleRange")
                .field("module_path", &self.module_path.join("::"))
                .field("start_offset", &format!("{:#010x}", self.start_offset))
                .field("end_offset", &format!("{:#010x}", self.end_offset))
                .finish()
        })
    }
}

/// Visitor for building namespace ranges efficiently
/// Only visits namespaces and uses depth-first traversal to capture ranges
#[derive(Default)]
struct ModuleRangeBuilder {
    module_ranges: Vec<ModuleRange>,
    modules: Module,
    last_seen_offset: usize, // Last offset seen during traversal
    namespace_stack: Vec<(String, Die, usize)>, // (path segment, die, start_offset)
}

impl<'db> DieVisitor<'db> for ModuleRangeBuilder {
    fn visit_cu<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        if is_rust_cu(&node.die, &node.unit_ref) {
            walker.visitor.namespace_stack.clear();
            let die = walker.get_die(node.die);
            let start_offset = die.offset();
            walker.visitor.last_seen_offset = start_offset;

            walker
                .visitor
                .namespace_stack
                .push((String::new(), die, start_offset));

            walker.walk_cu()?;

            // for the last range, we need to take the most recently inserted range
            // and extend it to the end of the CU
            let module_path = walker
                .visitor
                .namespace_stack
                .iter()
                .skip(1)
                .map(|(segment, _, _)| segment.clone())
                .collect::<Vec<_>>();

            let end_offset = start_offset + node.unit_ref.unit.header.unit_length();
            walker.visitor.module_ranges.push(ModuleRange {
                module_path,
                die,
                start_offset: walker.visitor.last_seen_offset,
                end_offset,
            });
        }
        Ok(())
    }

    fn visit_die<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        if node.die.tag() == gimli::DW_TAG_namespace {
            // Only visit namespaces, skip other DIEs
            Self::visit_namespace(walker, node)?;
        }
        Ok(())
    }

    fn visit_namespace<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        let module_name = get_string_attr(&node.die, gimli::DW_AT_name, &node.unit_ref)
            .unwrap()
            .unwrap();

        let die = walker.get_die(node.die);
        let start_offset = die.offset();

        if let Some((_, last_die, _)) = walker.visitor.namespace_stack.last() {
            if walker.visitor.last_seen_offset < start_offset {
                // If the last segment's start offset is before this one,
                // we need to close the previous namespace range
                let module_path = walker
                    .visitor
                    .namespace_stack
                    .iter()
                    .skip(1)
                    .map(|(segment, _, _)| segment.clone())
                    .collect::<Vec<_>>();

                let range = ModuleRange {
                    module_path,
                    die: *last_die,
                    start_offset: walker.visitor.last_seen_offset,
                    end_offset: start_offset,
                };

                tracing::trace!("Closing module range for {range:#?}",);
                walker.visitor.last_seen_offset = start_offset;

                // Use the current offset as the end (we'll update this with the next entry)
                walker.visitor.module_ranges.push(range);
            }
        }

        walker
            .visitor
            .namespace_stack
            .push((module_name.clone(), die, start_offset));

        let mut module = &mut walker.visitor.modules;
        // Traverse or create the module path
        // (skipping the first segment which is always empty)
        for (segment, _, _) in walker.visitor.namespace_stack.iter().skip(1) {
            module = module.modules.entry(segment.clone()).or_default();
        }

        // push this namespace to the Die entries
        module.entries.push(die);

        tracing::trace!("Visiting namespace: {module_name} at offset {start_offset:#010x}",);
        walker.walk_namespace()?;

        // When we're done walking this namespace, record its range
        if let Some(end_offset) = walker.peek_next_offset() {
            if let Some((path, _, _)) = walker.visitor.namespace_stack.pop() {
                let start = walker.visitor.last_seen_offset;
                if end_offset > start {
                    walker.visitor.last_seen_offset = end_offset;

                    let mut module_path = walker
                        .visitor
                        .namespace_stack
                        .iter()
                        .skip(1)
                        .map(|(segment, _, _)| segment.clone())
                        .collect::<Vec<_>>();

                    module_path.push(path);

                    let range = ModuleRange {
                        module_path,
                        start_offset: start,
                        end_offset,
                        die,
                    };
                    tracing::trace!("Walked module range: {range:#?}");
                    // Use the current offset as the end (we'll update this with the next entry)
                    walker.visitor.module_ranges.push(range);
                }
            }
        }

        Ok(())
    }
}

/// Build namespace ranges for efficient offset-based lookups
/// This is used to efficiently resolve type contexts without full indexing
#[salsa::tracked(returns(ref))]
pub fn module_index<'db>(db: &'db dyn DwarfDb, debug_file: DebugFile) -> ModuleIndex<'db> {
    let mut builder = ModuleRangeBuilder::default();
    if let Err(e) = walk_file(db, debug_file, &mut builder) {
        tracing::error!("Failed to walk file: {e}");
    }

    let ModuleRangeBuilder {
        module_ranges,
        modules,
        namespace_stack: _,
        last_seen_offset: _,
    } = builder;
    // Sort ranges by start offset and fix overlapping ranges
    let mut ranges = module_ranges;
    ranges.sort_by_key(|r| r.start_offset);

    ModuleIndex::new(db, ranges, modules)
}

pub fn get_containing_module(db: &dyn DwarfDb, die: Die) -> Option<Vec<String>> {
    let module_index = module_index(db, die.file);
    let die_offset = die.offset();
    module_index
        .find_by_offset(db, die_offset)
        .map(|range| range.module_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils;

    #[test]
    fn test_module_index() {
        let _guard = test_utils::init_tracing_and_insta();

        let platform = "aarch64-unknown-linux-gnu";
        let example_name = "examples/enums";

        let artifacts = test_utils::artifacts_dir(Some(platform));
        let db = test_utils::test_db(Some(platform));
        let db = &db;
        let binary = test_utils::load_binary(db, artifacts.join(example_name));

        let (debug_files, _) = crate::symbols::index_symbol_map(db, binary).unwrap();

        let mut output = String::new();

        for (_, file) in debug_files {
            let module_index = module_index(db, file);
            let file_name = file.name(db);

            let by_offset = module_index.by_offset(db);
            let by_name = module_index.by_name(db);
            output.push_str(&format!("\n=== {file_name} ===\n"));
            salsa::attach(db, || {
                output.push_str("===== By Offset =====\n\n");
                for range in by_offset {
                    output.push_str(&format!(
                        "{:#010x} - {:#010x}: {}\n",
                        range.start_offset,
                        range.end_offset,
                        range.module_path.join("::")
                    ));
                }

                fn print_module(module: &Module, indent: usize, output: &mut String) {
                    let indent_str = " ".repeat(indent);

                    for (name, submodule) in &module.modules {
                        output.push_str(&format!("{indent_str}{name}:\n"));
                        print_module(submodule, indent + 2, output);
                    }
                }

                output.push_str("\n===== By Name =====\n\n");

                // for modules, just print the nested names
                print_module(by_name, 0, &mut output);
            });

            // break;
        }

        insta::assert_snapshot!(output);
    }
}
