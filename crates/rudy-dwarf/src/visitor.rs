use super::loader::{DwarfReader, RawDie};
use super::unit::UnitRef;
use super::Die;
use crate::file::DebugFile;
use crate::DwarfDb;

/// Walker that drives the visitor through the DIE tree
pub struct DieWalker<'a, 'db, V> {
    pub db: &'db dyn DwarfDb,
    pub visitor: &'a mut V,

    /// The file being walked
    pub file: DebugFile,
    /// The compilation unit being walked
    unit_offset: gimli::UnitSectionOffset<usize>,
    pub unit_ref: &'a UnitRef<'a>,
    /// The entries cursor for the current unit
    // unit_ref: &'a UnitRef<'a>,
    cursor: gimli::EntriesCursor<'a, 'a, DwarfReader>,

    /// The current depth in the DIE tree
    current_depth: isize,
    /// The next peeked entry, if any
    /// Stored as (depth, RawDie)
    next_entry: Option<(isize, RawDie<'a>)>,
    depth: isize,
}

pub fn walk_file<'db, 'a, V: DieVisitor<'db>>(
    db: &'db dyn DwarfDb,
    file: DebugFile,
    visitor: &'a mut V,
) {
    tracing::trace!("Walking DWARF for file: {}", file.name(db));
    // Get the root compilation units for this file
    let roots = super::navigation::get_roots(db, file);

    for (unit_offset, unit_ref) in &roots {
        // Create a walker for each unit
        let mut walker = DieWalker {
            db,
            visitor: &mut *visitor,
            file,
            unit_offset: *unit_offset,
            unit_ref,
            cursor: unit_ref.entries(),
            current_depth: 0,
            next_entry: None,
            depth: 0,
        };

        // Walk the compilation unit
        tracing::trace!(
            "Walking CU: {:#x}",
            unit_offset.as_debug_info_offset().unwrap().0
        );
        walker.walk_unit();
    }
}

pub fn walk_die<'db, 'a, V: DieVisitor<'db>>(
    db: &'db dyn DwarfDb,
    die: Die<'db>,
    visitor: &'a mut V,
) -> anyhow::Result<()> {
    tracing::trace!("Walking DWARF for DIE: {}", die.print(db));

    die.with_entry_and_unit(db, |entry, unit_ref| {
        let cursor = unit_ref.entries_at_offset(entry.offset())?;
        let mut walker = DieWalker {
            db,
            visitor,
            file: die.file(db),
            unit_offset: die.cu_offset(db),
            unit_ref,
            cursor,
            current_depth: 0,
            next_entry: None,
            depth: 0,
        };
        let Some(die) = walker.next() else {
            tracing::error!("No entries found in DIE: {}", die.print(db));
            return Ok(());
        };
        V::visit_die(&mut walker, die, *unit_ref);
        Ok(())
    })?
}

impl<'a, 'db, V: DieVisitor<'db>> DieWalker<'a, 'db, V> {
    pub fn get_die(&self, raw: RawDie<'a>) -> Die<'db> {
        Die::new(self.db, self.file, self.unit_offset, raw.offset())
    }

    pub fn peek_next_offset(&mut self) -> Option<usize> {
        Some(self.peek()?.1.offset().0)
    }

    fn peek(&mut self) -> Option<(isize, RawDie<'a>)> {
        if self.next_entry.is_none() {
            match self.cursor.next_dfs() {
                Ok(Some((depth_delta, die))) => {
                    self.depth += depth_delta;
                    // Store the next entry for later use
                    self.next_entry = Some((self.depth, die.clone()));
                }
                Ok(None) => {
                    // No more entries to peek at
                    return None;
                }
                Err(e) => {
                    tracing::error!("Failed to parse DIE: {e}");
                    // we'll stop walking on error
                    return None;
                }
            }
        }

        self.next_entry.clone()
    }

    fn has_children(&mut self) -> bool {
        // The current DIE has children if the next entry is a child of the current DIE
        if let Some((depth, _)) = self.peek() {
            depth == self.current_depth + 1
        } else {
            // if there's no next entry, then there are no children
            false
        }
    }

    fn next(&mut self) -> Option<RawDie<'a>> {
        // if we have a peeked entry, return it
        if let Some((depth, die)) = self.next_entry.take() {
            debug_assert_eq!(
                depth, self.current_depth,
                "`next` must only be called when already at the correct depth"
            );
            return Some(die);
        }

        // otherwise, get the next entry from the cursor
        match self.cursor.next_dfs() {
            Ok(Some((delta, die))) => {
                debug_assert_eq!(
                    delta, 0,
                    "`next` must only be called when already at the correct depth"
                );
                Some(die.clone())
            }
            Ok(None) => None,
            Err(e) => {
                tracing::error!("Failed to parse DIE: {e}");
                None
            }
        }
    }

    fn next_sibling(&mut self) -> Option<RawDie<'a>> {
        match self.peek() {
            Some((depth, die)) if depth == self.current_depth => {
                tracing::trace!("got sibling {:#x} at depth {depth}", die.offset().0);
                // we have a sibling at the current depth, return it
                self.next_entry.take().map(|(_, die)| die)
            }
            // the next entry is a child of the current DIE, so we need to skip it
            Some((depth, _)) if depth > self.current_depth => {
                tracing::trace!(
                    "next entry is a descendent of {} at depth {depth}, skipping (and removing all siblings)",
                    self.current_depth
                );
                self.next_entry.take();
                while self.cursor.next_sibling().ok().flatten().is_some() {
                    // keep skipping siblings until we find one at the current depth
                }
                self.next_sibling()
            }
            // either we have no next entry, or the next entry is not a sibling
            Some((depth, _)) => {
                tracing::trace!(
                    "next entry is a sibling of a parent at depth {depth} < {}, we're out of siblings",
                    self.current_depth
                );
                None
            }
            _ => {
                tracing::trace!("no next entry, returning None");
                None
            }
        }
    }

    pub fn walk_unit(&mut self) {
        let Some(root) = self.next() else {
            // empty tree -- nothing to walk
            tracing::info!("No entries found in DWARF tree");
            return;
        };

        // first entry _should_ be the root DIE -- the compilation unit
        let tag = root.tag();
        if tag != gimli::DW_TAG_compile_unit {
            tracing::error!("Expected root DIE to be a compilation unit, found: {tag}");
            return;
        }

        let unit_ref = *self.unit_ref;
        tracing::trace!("Visiting CU: {:#x}", root.offset().0);
        V::visit_cu(self, root, unit_ref);
    }

    pub fn walk_children(&mut self) {
        let current_offset = self.cursor.current().map_or(0, |c| c.offset().0);

        if !self.has_children() {
            return;
        }

        self.current_depth += 1;
        tracing::trace!(
            "Walking children of entry: {current_offset:#x} at depth: {}",
            self.current_depth
        );

        // walk the siblings at this depth
        while let Some(next) = self.next_sibling() {
            // continue walking siblings
            V::visit_die(self, next, *self.unit_ref);
        }
        tracing::trace!("Finished walking children of entry: {current_offset:#x}");
        self.current_depth -= 1;
    }

    pub fn walk_cu(&mut self) {
        self.walk_children();
    }

    pub fn walk_namespace(&mut self) {
        self.walk_children();
    }
}

/// Visitor trait for walking DWARF DIE trees
pub trait DieVisitor<'db>: Sized {
    fn visit_cu<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _die: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_cu();
    }

    /// Called for each DIE entry
    fn visit_die<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        die: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        tracing::trace!("Visiting DIE: {:#010x}", die.offset().0,);
        // Default: dispatch to specific visit methods based on tag
        match die.tag() {
            gimli::DW_TAG_namespace => Self::visit_namespace(walker, die, unit_ref),
            gimli::DW_TAG_subprogram => Self::visit_function(walker, die, unit_ref),
            gimli::DW_TAG_structure_type => Self::visit_struct(walker, die, unit_ref),
            gimli::DW_TAG_enumeration_type => Self::visit_enum(walker, die, unit_ref),
            gimli::DW_TAG_variable => Self::visit_variable(walker, die, unit_ref),
            gimli::DW_TAG_formal_parameter => Self::visit_parameter(walker, die, unit_ref),
            gimli::DW_TAG_base_type => Self::visit_base_type(walker, die, unit_ref),
            gimli::DW_TAG_pointer_type => Self::visit_pointer_type(walker, die, unit_ref),
            gimli::DW_TAG_array_type => Self::visit_array_type(walker, die, unit_ref),
            gimli::DW_TAG_lexical_block => Self::visit_lexical_block(walker, die, unit_ref),
            gimli::DW_TAG_union_type => {
                Self::visit_union_type(walker, die, unit_ref);
            }
            gimli::DW_TAG_subroutine_type => {
                // these don't seem to contain much, so we'll skip
            }
            gimli::DW_TAG_member
            | gimli::DW_TAG_template_type_parameter
            | gimli::DW_TAG_variant_part
            | gimli::DW_TAG_subrange_type
            | gimli::DW_TAG_enumerator
            | gimli::DW_TAG_inlined_subroutine => {
                // these should typically be visited explicitly
                // as part of visiting the parent
            }
            _ => {
                tracing::debug!(
                    "Unhandled DIE tag: {} {}",
                    die.tag(),
                    walker.get_die(die).format_with_location(walker.db, "")
                );
            }
        }
    }

    /// Visit a namespace
    fn visit_namespace<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_namespace();
    }

    /// Visit a function/subprogram
    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a struct type
    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit an enum type
    fn visit_enum<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a variable
    fn visit_variable<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a variable
    fn visit_parameter<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a base type
    fn visit_base_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a pointer type
    fn visit_pointer_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a array type
    fn visit_array_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }
    fn visit_union_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    fn visit_lexical_block<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        _unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }
}

#[cfg(test)]
mod test {
    use itertools::Itertools;

    use crate::utils::get_string_attr;

    struct TestVisitor {
        pub visited: Vec<String>,
    }

    impl<'db> super::DieVisitor<'db> for TestVisitor {
        fn visit_cu<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            die: crate::loader::RawDie<'a>,
            unit_ref: crate::unit::UnitRef<'a>,
        ) {
            Self::visit_die(walker, die, unit_ref);
        }

        fn visit_die<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            die: crate::loader::RawDie<'a>,
            unit_ref: crate::unit::UnitRef<'a>,
        ) {
            let offset = die.offset().0;
            let padding = std::iter::repeat_n(" ", 2 * walker.current_depth as usize).join("");

            let tag = die.tag();
            walker
                .visitor
                .visited
                .push(format!("{offset:#010x}: {padding}{tag}"));
            // Default: dispatch to specific visit methods based on tag
            match die.tag() {
                gimli::DW_TAG_namespace => Self::visit_namespace(walker, die, unit_ref),
                gimli::DW_TAG_subprogram => Self::visit_function(walker, die, unit_ref),
                gimli::DW_TAG_structure_type => Self::visit_struct(walker, die, unit_ref),
                gimli::DW_TAG_enumeration_type => Self::visit_enum(walker, die, unit_ref),
                gimli::DW_TAG_variable => Self::visit_variable(walker, die, unit_ref),
                gimli::DW_TAG_base_type => Self::visit_base_type(walker, die, unit_ref),
                gimli::DW_TAG_compile_unit => {}
                _ => {}
            }
            walker.walk_children();
        }
    }

    #[test]
    fn test_visitor() {
        crate::test_utils::init_tracing();
        let small_file = crate::test_utils::root_artifacts_dir()
            .join("x86_64-unknown-linux-gnu")
            .join("small");
        let db = crate::test_utils::test_db(None);
        let db = &db;
        let binary = crate::test_utils::load_binary(db, small_file);
        let mut visitor = TestVisitor {
            visited: Vec::new(),
        };

        let debug_file = crate::file::DebugFile::new(db, binary.file(db), false);

        super::walk_file(db, debug_file, &mut visitor);

        // Check that we visited the expected entries
        assert!(!visitor.visited.is_empty(), "No entries were visited");
        insta::assert_snapshot!(visitor.visited.join("\n"));
    }

    #[derive(Default)]
    struct ModuleFunctionVisitor {
        pub path: Vec<String>,
        pub functions: Vec<String>,
    }

    impl<'db> super::DieVisitor<'db> for ModuleFunctionVisitor {
        fn visit_struct<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            entry: crate::loader::RawDie<'a>,
            unit_ref: crate::unit::UnitRef<'a>,
        ) {
            let name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
                .unwrap()
                .unwrap_or_else(|| "<unnamed>".to_string());
            walker.visitor.path.push(name);
            walker.walk_children();
            walker.visitor.path.pop();
        }

        fn visit_namespace<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            entry: crate::loader::RawDie<'a>,
            unit_ref: crate::unit::UnitRef<'a>,
        ) {
            let name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
                .unwrap()
                .unwrap_or_else(|| "<unnamed>".to_string());
            walker.visitor.path.push(name);
            walker.walk_namespace();
            walker.visitor.path.pop();
        }

        fn visit_function<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            entry: crate::loader::RawDie<'a>,
            unit_ref: crate::unit::UnitRef<'a>,
        ) {
            let name = get_string_attr(&entry, gimli::DW_AT_name, &unit_ref)
                .unwrap()
                .unwrap_or_else(|| "<unnamed>".to_string());
            let mut path = walker.visitor.path.clone();
            path.push(name);
            walker.visitor.functions.push(path.join("::"));
        }
    }

    #[test]
    fn methods_and_functions() {
        crate::test_utils::init_tracing();

        let small_file = crate::test_utils::root_artifacts_dir()
            .join("x86_64-unknown-linux-gnu")
            .join("small");

        // Create a test database
        let db = crate::test_utils::test_db(None);
        let db = &db;

        // Create a test binary
        let binary = crate::test_utils::load_binary(db, small_file);

        let (debug_files, _) = crate::symbols::index_symbol_map(db, binary).unwrap();

        let mut visitor = ModuleFunctionVisitor::default();
        for (_, file) in debug_files {
            super::walk_file(db, file, &mut visitor);
        }

        visitor.functions.retain(|f| f != "<unnamed>");
        visitor.functions.dedup();
        visitor.functions.sort();

        // Check that we visited the expected entries
        assert!(!visitor.functions.is_empty(), "No functions were visited");
        insta::assert_snapshot!(visitor.functions.join("\n"));
    }
}
