use super::loader::{DwarfReader, RawDie};
use super::unit::UnitRef;
use super::{CompilationUnitId, Die};
use crate::database::Db;
use crate::dwarf::utils::pretty_print_die_entry;
use crate::file::File;
use gimli::DebuggingInformationEntry;

/// Walker that drives the visitor through the DIE tree
pub struct DieWalker<'a, 'db, V> {
    pub db: &'db dyn Db,
    pub visitor: &'a mut V,

    /// The file being walked
    pub file: File,
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

pub fn walk_file<'db, 'a, V: DieVisitor<'db>>(db: &'db dyn Db, file: File, visitor: &'a mut V) {
    tracing::trace!("Walking DWARF for file: {}", file.path(db));
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

impl<'a, 'db, V: DieVisitor<'db>> DieWalker<'a, 'db, V> {
    pub fn get_die(&self, raw: RawDie<'a>) -> Die<'db> {
        Die::new(self.db, self.file, self.unit_offset, raw.offset())
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
                    self.db.report_error(format!("Failed to parse DIE: {e}"));
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
                self.db.report_error(format!("Failed to parse DIE: {e}"));
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
            self.db
                .report_info("No entries found in DWARF tree".to_string());
            return;
        };

        // first entry _should_ be the root DIE -- the compilation unit
        let tag = root.tag();
        if tag != gimli::DW_TAG_compile_unit {
            self.db.report_error(format!(
                "Expected root DIE to be a compilation unit, found: {tag:?}"
            ));
            return;
        }

        let unit_ref = self.unit_ref.clone();
        tracing::trace!("Visiting CU: {:#x}", root.offset().0);
        V::visit_cu(self, root, unit_ref);
    }

    fn walk_children(&mut self) {
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
            // tracing::trace!(
            //     "Visiting DIE: {:#x} {} {}",
            //     next.offset().0,
            //     next.tag(),
            //     pretty_print_die_entry(&next, &self.unit_ref)
            // );
            // continue walking siblings
            V::visit_die(self, next, self.unit_ref.clone());
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

    pub fn walk_function(&mut self) {
        self.walk_children();
    }

    pub fn walk_struct(&mut self) {
        self.walk_children();
    }

    // pub fn walk_namespace(&mut self, die: RawDie<'a>, unit_ref: UnitRef<'a>) {
    //     if let Some(mut tree) = self
    //         .unit_ref
    //         .entries_tree(Some(die.die_offset()))
    //         .inspect_err(|e| {
    //             self.db
    //                 .report_critical(format!("Failed to parse child nodes: {e}"));
    //         })
    //         .ok()
    //     {
    //         let Some(tree_root) = tree
    //             .root()
    //             .inspect_err(|e| {
    //                 self.db
    //                     .report_critical(format!("Failed to parse child nodes: {e}"));
    //             })
    //             .ok()
    //         else {
    //             return;
    //         };

    //         for child in tree_root.children() {
    //             let child_die = RawDie::new(self.db, die.file(), die.cu_offset(), child.offset());
    //             self.visitor.visit_namespace(self.db, &child_die, &child);
    //         }
    //     }

    //     // let Some(mut tree) = self
    //     //     .unit_ref
    //     //     .entries_tree(Some(self.die_offset(db)))
    //     //     .inspect_err(|e| {
    //     //         db.report_critical(format!("Failed to parse child nodes: {e}"));
    //     //     })
    //     //     .ok()
    //     // else {
    //     //     return children;
    //     // };
    // }

    // /// Walk a compilation unit with the given visitor
    // pub fn walk_unit(
    //     &self,
    //     unit: CompilationUnitId<'a>,
    //     visitor: &mut V,
    // ) -> Result<(), gimli::Error> {
    //     let unit_ref = unit.unit_ref(self.db).ok_or(gimli::Error::InvalidInput)?;
    //     let file = unit.file(self.db);
    //     let cu_offset = unit.offset(self.db);

    //     let mut entries = unit_ref.entries();
    //     self.walk_entries(&mut entries, &unit_ref, file, cu_offset, visitor, 0)
    // }

    // /// Internal recursive walker
    // fn walk_entries(
    //     &self,
    //     entries: &mut gimli::EntriesCursor<'_>,
    //     unit_ref: UnitRef<'_>,
    //     file: crate::file::File,
    //     cu_offset: gimli::DebugInfoOffset,
    //     visitor: &mut V,
    //     depth: usize,
    // ) -> Result<(), gimli::Error> {
    //     loop {
    //         let Some((_, entry)) = entries.next_dfs()? else {
    //             break;
    //         };

    //         // Create Die handle
    //         let die = Die::new(self.db, file, cu_offset, entry.offset());

    //         // Visit the DIE
    //         let control = visitor.visit_die(self.db, &die, &entry);

    //         match control {
    //             VisitorControl::Continue => {
    //                 if entry.has_children() {
    //                     visitor.enter_scope(self.db, &die, &entry);
    //                     // Continue traversal - next_dfs will handle children
    //                 }
    //             }
    //             VisitorControl::SkipChildren => {
    //                 // Skip to next sibling
    //                 entries.skip_attributes()?;
    //                 while entries.current().is_some() && entries.current().unwrap().1 > depth {
    //                     entries.next_dfs()?;
    //                 }
    //             }
    //             VisitorControl::Break => {
    //                 return Ok(());
    //             }
    //         }

    //         // Check if we're leaving a scope
    //         if let Some((next_depth, _)) = entries.current() {
    //             if next_depth < depth && depth > 0 {
    //                 // We're about to leave this scope
    //                 visitor.leave_scope(self.db, &die, &entry);
    //             }
    //         }
    //     }

    //     Ok(())
    // }
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
        // Default: dispatch to specific visit methods based on tag
        match die.tag() {
            gimli::DW_TAG_namespace => Self::visit_namespace(walker, die, unit_ref),
            gimli::DW_TAG_subprogram => Self::visit_function(walker, die, unit_ref),
            gimli::DW_TAG_structure_type => Self::visit_struct(walker, die, unit_ref),
            gimli::DW_TAG_enumeration_type => Self::visit_enum(walker, die, unit_ref),
            gimli::DW_TAG_variable => Self::visit_variable(walker, die, unit_ref),
            gimli::DW_TAG_base_type => Self::visit_base_type(walker, die, unit_ref),
            _ => {
                tracing::warn!(
                    "Unhandled DIE tag: {:?} at offset {:#x}",
                    die.tag(),
                    die.offset().0
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
        walker.walk_function();
    }

    /// Visit a struct type
    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit an enum type
    fn visit_enum<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a variable
    fn visit_variable<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    /// Visit a base type
    fn visit_base_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
        walker.walk_children();
    }

    // /// Called when entering a new scope (before visiting children)
    // fn enter_scope(walker: &mut DieWalker<'a, 'db, Self>, _entry: RawDie<'a>, unit_ref: UnitRef<'a>) {}

    // /// Called when leaving a scope (after visiting children)
    // fn leave_scope(walker: &mut DieWalker<'a, 'db, Self>, _entry: RawDie<'a>, unit_ref: UnitRef<'a>) {}
}

#[cfg(test)]
mod test {
    use itertools::Itertools;
    use tracing_subscriber::EnvFilter;

    use crate::dwarf::utils::get_string_attr;

    struct TestVisitor {
        pub visited: Vec<String>,
    }

    impl<'db> super::DieVisitor<'db> for TestVisitor {
        fn visit_cu<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            die: crate::dwarf::loader::RawDie<'a>,
            unit_ref: crate::dwarf::unit::UnitRef<'a>,
        ) {
            Self::visit_die(walker, die, unit_ref);
            walker.walk_cu();
        }

        fn visit_die<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            die: crate::dwarf::loader::RawDie<'a>,
            unit_ref: crate::dwarf::unit::UnitRef<'a>,
        ) {
            let offset = die.offset().0;
            // if offset > 0x000000a6 {
            //     // Skip entries after a certain offset for testing
            //     return;
            // }
            let padding = std::iter::repeat(" ")
                .take(2 * walker.current_depth as usize)
                .join("");

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
                _ => {
                    walker.walk_children();
                }
            }
        }
    }

    #[test]
    fn test_visitor() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .try_init();
        let db = crate::database::DebugDatabaseImpl::new().unwrap();
        // Load a test DWARF file
        // db.analyze_file("benches/test_binaries/small")

        let file = super::File::build(
            &db,
            "benches/test_binaries/small.small.f3ea0c7117bb9874-cgu.0.rcgu.o".to_string(),
            None,
        )
        .unwrap();
        let mut visitor = TestVisitor {
            visited: Vec::new(),
        };

        super::walk_file(&db, file, &mut visitor);

        // Check that we visited the expected entries
        assert!(!visitor.visited.is_empty(), "No entries were visited");
        insta::assert_snapshot!(visitor.visited.join("\n"));

        let file = super::File::build(
            &db,
            "benches/test_binaries/medium.medium.b63b38f5b684d51-cgu.0.rcgu.o".to_string(),
            None,
        )
        .unwrap();
        let mut visitor = TestVisitor {
            visited: Vec::new(),
        };

        super::walk_file(&db, file, &mut visitor);

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
        fn visit_namespace<'a>(
            walker: &mut super::DieWalker<'a, 'db, Self>,
            entry: crate::dwarf::loader::RawDie<'a>,
            unit_ref: crate::dwarf::unit::UnitRef<'a>,
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
            entry: crate::dwarf::loader::RawDie<'a>,
            unit_ref: crate::dwarf::unit::UnitRef<'a>,
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
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .try_init();
        let db = crate::database::DebugDatabaseImpl::new().unwrap();
        // Load a test DWARF file
        // db.analyze_file("benches/test_binaries/small")

        let file = super::File::build(
            &db,
            "benches/test_binaries/small.small.f3ea0c7117bb9874-cgu.0.rcgu.o".to_string(),
            None,
        )
        .unwrap();
        let mut visitor = ModuleFunctionVisitor::default();

        super::walk_file(&db, file, &mut visitor);

        // Check that we visited the expected entries
        assert!(!visitor.functions.is_empty(), "No functions were visited");
        insta::assert_snapshot!(visitor.functions.join("\n"));

        // let file = super::File::build(
        //     &db,
        //     "benches/test_binaries/medium.medium.b63b38f5b684d51-cgu.0.rcgu.o".to_string(),
        //     None,
        // )
        // .unwrap();
        // let mut visitor = ModuleFunctionVisitor::default();

        // super::walk_file(&db, file, &mut visitor);

        // // Check that we visited the expected entries
        // assert!(!visitor.functions.is_empty(), "No functions were visited");
        // insta::assert_snapshot!(visitor.functions.join("\n"));
    }
}
