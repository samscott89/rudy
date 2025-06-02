use super::loader::{DwarfReader, RawDie};
use super::unit::UnitRef;
use super::{CompilationUnitId, Die};
use crate::database::Db;
use crate::file::File;
use gimli::DebuggingInformationEntry;

/// Control flow for visitor traversal
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VisitorControl {
    /// Continue traversing the tree (visit children)
    Continue,
    /// Skip children of current node, continue with siblings
    SkipChildren,
    /// Stop traversal entirely
    Break,
}

/// Walker that drives the visitor through the DIE tree
pub struct DieWalker<'a, 'db, V> {
    pub db: &'db dyn Db,
    pub visitor: &'a mut V,

    /// The file being walked
    file: File,
    /// The compilation unit being walked
    unit_offset: gimli::UnitSectionOffset<usize>,
    pub unit_ref: &'a UnitRef<'a>,
    /// The entries cursor for the current unit
    // unit_ref: &'a UnitRef<'a>,
    cursor: gimli::EntriesCursor<'a, 'a, DwarfReader>,
}

pub fn walk_file<'db, 'a, V: DieVisitor<'db>>(db: &'db dyn Db, file: File, visitor: &'a mut V) {
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
        };

        // Walk the compilation unit
        walker.walk_unit();
    }
}

impl<'a, 'db, V: DieVisitor<'db>> DieWalker<'a, 'db, V> {
    pub fn get_die(&self, raw: RawDie<'a>) -> Die<'db> {
        Die::new(self.db, self.file, self.unit_offset, raw.offset())
    }

    fn next_dfs(&mut self) -> Option<(isize, RawDie<'a>)> {
        match self.cursor.next_dfs() {
            Ok(Some((offset, die))) => Some((offset, die.clone())),
            Ok(None) => None,
            Err(e) => {
                self.db.report_error(format!("Failed to parse DIE: {e}"));
                None
            }
        }
    }

    fn next_sibling(&mut self) -> Option<RawDie<'a>> {
        match self.cursor.next_sibling() {
            Ok(Some(die)) => Some(die.clone()),
            Ok(None) => None,
            Err(e) => {
                self.db.report_error(format!("Failed to parse DIE: {e}"));
                None
            }
        }
    }

    pub fn walk_unit(&mut self) {
        let Some((delta, root)) = self.next_dfs() else {
            // empty tree -- nothing to walk
            self.db
                .report_info("No entries found in DWARF tree".to_string());
            return;
        };

        debug_assert!(delta == 0, "Expected root DIE to be at depth 0");

        // first entry _should_ be the root DIE -- the compilation unit
        let tag = root.tag();
        if tag != gimli::DW_TAG_compile_unit {
            self.db.report_error(format!(
                "Expected root DIE to be a compilation unit, found: {tag:?}"
            ));
            return;
        }

        let unit_ref = self.unit_ref.clone();
        V::visit_cu(self, root, unit_ref);
    }

    fn walk_generic_children(&mut self) {
        // walk the children of the compilation unit
        if let Some((delta, first_child)) = self.next_dfs() {
            debug_assert_eq!(delta, 1, "Expected first child to be at depth 1");
            V::visit_die(self, first_child, self.unit_ref.clone());

            while let Some(next) = self.next_sibling() {
                // continue walking siblings
                V::visit_die(self, next, self.unit_ref.clone());
            }
        }
    }

    pub fn walk_cu(&mut self) {
        self.walk_generic_children();
    }

    pub fn walk_namespace(&mut self) {
        self.walk_generic_children();
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
            _ => {}
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
        unit_ref: UnitRef<'a>,
    ) {
    }

    /// Visit a struct type
    fn visit_struct<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
    }

    /// Visit an enum type
    fn visit_enum<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
    }

    /// Visit a variable
    fn visit_variable<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
    }

    /// Visit a base type
    fn visit_base_type<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        _entry: RawDie<'a>,
        unit_ref: UnitRef<'a>,
    ) {
    }

    // /// Called when entering a new scope (before visiting children)
    // fn enter_scope(walker: &mut DieWalker<'a, 'db, Self>, _entry: RawDie<'a>, unit_ref: UnitRef<'a>) {}

    // /// Called when leaving a scope (after visiting children)
    // fn leave_scope(walker: &mut DieWalker<'a, 'db, Self>, _entry: RawDie<'a>, unit_ref: UnitRef<'a>) {}
}
