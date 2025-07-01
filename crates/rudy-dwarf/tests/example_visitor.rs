//! Tests for examining DWARF structure to understand method organization

use rudy_dwarf::{
    parser::{
        combinators::all,
        primitives::{die_offset, name, tag},
        Parser,
    },
    visitor::{walk_file, DieVisitor, DieWalker, VisitorNode},
};

pub mod common;

use common::{load_binary, test_db};

use anyhow::Result;

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
    fn add_entry(&mut self, mut entry: StructureEntry) {
        if self.should_visit() {
            entry.depth = self.depth;
            entry.module_path = self.current_path.clone();
            self.structure.push(entry);
        }
    }

    /// Generate a formatted string representation of the structure
    pub fn format_structure(&self) -> String {
        let mut output = String::new();

        if self.structure.is_empty() {
            output.push_str("No structure found");
            if let Some(target) = &self.target_module {
                output.push_str(&format!(" for module '{target}'",));
            }
            output.push('\n');
            return output;
        }

        for entry in &self.structure {
            let StructureEntry {
                depth,
                tag,
                name,
                module_path,
                offset,
            } = entry;
            let indent = "  ".repeat(*depth);
            let name_part = name.as_ref().map(|n| format!(" '{n}'")).unwrap_or_default();
            let module_part = if module_path.is_empty() {
                String::new()
            } else {
                format!(" ({})", module_path.join("::"))
            };

            output.push_str(&format!(
                "{offset}:  {indent}{tag}{name_part} @ {module_part}\n",
            ));
        }

        output
    }
}

fn entry_parser<'db>() -> impl Parser<'db, StructureEntry> {
    all((tag(), name(), die_offset())).map(|(tag, name, offset)| StructureEntry {
        depth: 0, // Depth will be set during walking
        tag: tag.to_string().replace("DW_TAG_", ""),
        name,
        module_path: Vec::new(), // Will be filled during walking
        offset: format!("{offset:#010x}"),
    })
}

impl<'db> DieVisitor<'db> for TestVisitor {
    fn visit_namespace<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> Result<()> {
        if let Ok(Some(namespace_name)) = node.name() {
            walker.visitor.current_path.push(namespace_name.clone());

            let entry = walker.parse(node, entry_parser())?;

            walker.visitor.add_entry(entry);

            walker.visitor.depth += 1;
            walker.walk_namespace()?;
            walker.visitor.depth -= 1;

            walker.visitor.current_path.pop();
        }
        Ok(())
    }

    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> Result<()> {
        let entry = walker.parse(node, entry_parser())?;

        walker.visitor.add_entry(entry);

        walker.visitor.depth += 1;
        walker.walk_children()?;
        walker.visitor.depth -= 1;
        Ok(())
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        entry: VisitorNode<'a>,
    ) -> Result<()> {
        let entry = walker.parse(entry, entry_parser())?;

        walker.visitor.add_entry(entry);

        walker.visitor.depth += 1;
        walker.walk_children()?;
        walker.visitor.depth -= 1;
        Ok(())
    }

    fn visit_enum<'a>(walker: &mut DieWalker<'a, 'db, Self>, node: VisitorNode<'a>) -> Result<()> {
        let entry = walker.parse(node, entry_parser())?;
        walker.visitor.add_entry(entry);

        walker.visitor.depth += 1;
        walker.walk_children()?;
        walker.visitor.depth -= 1;
        Ok(())
    }

    fn visit_variable<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> Result<()> {
        let entry = walker.parse(node, entry_parser())?;
        walker.visitor.add_entry(entry);
        Ok(())
    }

    fn visit_parameter<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> Result<()> {
        let entry = walker.parse(node, entry_parser())?;
        walker.visitor.add_entry(entry);
        Ok(())
    }
}

#[test]
fn dwarf_outline_examples() {
    let _guard = test_utils::init_tracing_and_insta();

    // Build the method_discovery example first
    let artifact_dir = test_utils::artifacts_dir(None);

    let db = test_db(None);
    let db = &db;

    for name in ["method_discovery", "simple_test", "lldb_demo"] {
        let path = artifact_dir.join(name);
        if !path.exists() {
            panic!(
                "Example binary not found at: {}. Please run `cargo xtask build-examples` first.",
                path.display()
            );
        }

        tracing::info!("Examining DWARF structure for: {name}");

        let binary = load_binary(db, &path);
        let (debug_files, _) =
            rudy_dwarf::symbols::index_symbol_map(db, binary).expect("Failed to index debug files");

        // Get debug files to examine
        let mut all_structure = String::new();

        for debug_file in debug_files.values() {
            let file_name = debug_file.name(db);

            // Only examine files that likely contain our method_discovery code
            tracing::info!("Examining DWARF structure for: {file_name}",);

            let mut visitor = TestVisitor::new_for_module(name);
            walk_file(db, *debug_file, &mut visitor).unwrap();

            let structure = visitor.format_structure();
            if !structure.trim().is_empty() && !structure.contains("No structure found") {
                all_structure.push_str(&format!("\n=== {file_name} ===\n",));
                all_structure.push_str(&structure);
            }
        }

        // Create snapshot of the structure
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown_file");
        insta::assert_snapshot!(name, all_structure);
    }
}
