//! Tests for examining DWARF structure to understand method organization

use std::path::PathBuf;

use rudy_dwarf::{
    test_utils::load_binary,
    visitor::{walk_file, DieVisitor, DieWalker},
};

pub mod common;

use common::test_db;

/// A test visitor for examining DWARF structure in a human-readable format
/// Useful for understanding how different Rust constructs are represented in DWARF
#[derive(Default)]
pub struct TestVisitor {
    /// Current namespace path while walking
    current_path: Vec<String>,
    /// Collected structure information
    pub structure: Vec<StructureEntry>,
    /// Target module to filter to (e.g., "method_discovery")
    target_module: Option<String>,
    /// Current depth for indentation
    depth: usize,
}

/// Represents a single entry in the DWARF structure
#[derive(Debug, Clone)]
pub struct StructureEntry {
    pub depth: usize,
    pub tag: String,
    pub name: Option<String>,
    pub module_path: Vec<String>,
    pub offset: String,
}

impl TestVisitor {
    /// Create a new test visitor that filters to a specific module
    pub fn new_for_module(module_name: &str) -> Self {
        Self {
            target_module: Some(module_name.to_string()),
            ..Default::default()
        }
    }

    /// Check if we're currently in the target module (or should visit everything)
    fn should_visit(&self) -> bool {
        match &self.target_module {
            None => true, // Visit everything if no filter
            Some(target) => self.current_path.iter().any(|segment| segment == target),
        }
    }

    /// Add an entry to the structure
    fn add_entry(&mut self, tag: gimli::DwTag, name: Option<String>, offset: gimli::UnitOffset) {
        if self.should_visit() {
            self.structure.push(StructureEntry {
                depth: self.depth,
                tag: format!("{:?}", tag),
                name,
                module_path: self.current_path.clone(),
                offset: format!("{:#x}", offset.0),
            });
        }
    }

    /// Generate a formatted string representation of the structure
    pub fn format_structure(&self) -> String {
        let mut output = String::new();

        if self.structure.is_empty() {
            output.push_str("No structure found");
            if let Some(target) = &self.target_module {
                output.push_str(&format!(" for module '{}'", target));
            }
            output.push('\n');
            return output;
        }

        for entry in &self.structure {
            let indent = "  ".repeat(entry.depth);
            let name_part = entry
                .name
                .as_ref()
                .map(|n| format!(" '{}'", n))
                .unwrap_or_default();
            let module_part = if entry.module_path.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.module_path.join("::"))
            };

            output.push_str(&format!(
                "{}{}{}{} @ {}\n",
                indent, entry.tag, name_part, module_part, entry.offset
            ));
        }

        output
    }
}

impl<'db> DieVisitor<'db> for TestVisitor {
    fn visit_cu<'a>(walker: &mut DieWalker<'a, 'db, Self>, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
        // Only visit Rust compilation units
        if is_rust_cu(walker.db, &die, &unit_ref) {
            walker.visitor.add_entry(die.tag(), None, die.offset());
            walker.walk_cu();
        }
    }

    fn visit_namespace<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        if let Ok(Some(namespace_name)) = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref) {
            walker.visitor.current_path.push(namespace_name.clone());
            walker
                .visitor
                .add_entry(entry.tag(), Some(namespace_name), entry.offset());

            walker.visitor.depth += 1;
            walker.walk_namespace();
            walker.visitor.depth -= 1;

            walker.visitor.current_path.pop();
        }
    }

    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        if let Ok(Some(struct_name)) = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref) {
            walker
                .visitor
                .add_entry(entry.tag(), Some(struct_name), entry.offset());

            walker.visitor.depth += 1;
            walker.walk_children();
            walker.visitor.depth -= 1;
        }
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .ok()
            .flatten()
            .or_else(|| {
                // Try linkage name if regular name is not available
                get_string_attr(&entry, gimli::DW_AT_linkage_name, &unit_ref)
                    .ok()
                    .flatten()
            });

        walker.visitor.add_entry(entry.tag(), name, entry.offset());

        walker.visitor.depth += 1;
        walker.walk_children();
        walker.visitor.depth -= 1;
    }

    fn visit_enum<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        if let Ok(Some(enum_name)) = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref) {
            walker
                .visitor
                .add_entry(entry.tag(), Some(enum_name), entry.offset());

            walker.visitor.depth += 1;
            walker.walk_children();
            walker.visitor.depth -= 1;
        }
    }

    fn visit_variable<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let var_name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .ok()
            .flatten();
        walker
            .visitor
            .add_entry(entry.tag(), var_name, entry.offset());
    }

    fn visit_parameter<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        let param_name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
            .ok()
            .flatten();
        walker
            .visitor
            .add_entry(entry.tag(), param_name, entry.offset());
    }
}

fn walk_debug_files(path: PathBuf) {
    let db = test_db(None);
    let db = &db;
    let binary = load_binary(db, path);
    let (debug_files, _) =
        rudy_dwarf::symbols::index_symbol_map(db, binary).expect("Failed to index debug files");

    // Get debug files to examine
    let mut all_structure = String::new();

    for debug_file in debug_files.values() {
        let file_name = debug_file.name(db);

        // Only examine files that likely contain our method_discovery code
        if file_name.contains("method_discovery") {
            tracing::info!("Examining DWARF structure for: {}", file_name);

            let mut visitor = TestVisitor::new_for_module("method_discovery");
            walk_file(db, *debug_file, &mut visitor);

            let structure = visitor.format_structure();
            if !structure.trim().is_empty() && !structure.contains("No structure found") {
                all_structure.push_str(&format!("\n=== {} ===\n", file_name));
                all_structure.push_str(&structure);
            }
        }
    }

    // Create snapshot of the structure
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown_file");
    insta::assert_snapshot!(name, all_structure);
}

#[test]
fn dwarf_outline_examples() {
    let _guard = test_utils::init_tracing_and_insta();

    // Build the method_discovery example first
    let artifact_dir = test_utils::artifacts_dir(None);

    for name in ["method_discovery", "simple_test", "lldb_demo"] {
        let path = artifact_dir.join(name).join("method_discovery");
        if !path.exists() {
            panic!(
                "Example binary not found at: {}. Please run `cargo xtask build-examples` first.",
                path.display()
            );
        }
        walk_debug_files(path);
    }
}
