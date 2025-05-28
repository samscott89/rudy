//! Core DWARF entity types and their navigation methods

use gimli::UnitSectionOffset;

use super::{
    CompilationUnitId,
    loader::{DwarfReader, Offset, RawDie, UnitRef},
    utils::{file_entry_to_path, parse_die_string_attribute},
};
use crate::file::FileId;
use crate::{database::Db, file::SourceFile};

/// References a specific DWARF debugging information entry
#[salsa::interned]
pub struct Die<'db> {
    pub file: FileId<'db>,
    pub cu_offset: UnitSectionOffset<usize>,
    pub die_offset: Offset,
}

impl<'db> Die<'db> {
    // GROUP 1: Core Identity (Keep - no dependencies)
    pub fn as_path_ref(&self, db: &'db dyn Db) -> String {
        let path = self.file(db).full_path(db);
        let cu_offset = match self.cu_offset(db) {
            UnitSectionOffset::DebugInfoOffset(debug_info_offset) => debug_info_offset.0,
            UnitSectionOffset::DebugTypesOffset(debug_types_offset) => debug_types_offset.0,
        };
        let die_offset = self.die_offset(db).0;
        format!("{path}:{cu_offset:#x}:{die_offset:#x}")
    }

    pub fn cu(&self, db: &'db dyn Db) -> CompilationUnitId<'db> {
        CompilationUnitId::new(db, self.file(db), self.cu_offset(db))
    }

    // GROUP 2: High-Cohesion Navigation + Basic Attributes (Keep - used together 90% of time)
    pub fn children(&self, db: &'db dyn Db) -> Vec<Die<'db>> {
        let mut children = vec![];

        let Some(unit_ref) = self.unit_ref(db) else {
            return children;
        };

        let Some(mut tree) = unit_ref
            .entries_tree(Some(self.die_offset(db)))
            .inspect_err(|e| {
                db.report_critical(format!("Failed to parse child nodes: {e}"));
            })
            .ok()
        else {
            return children;
        };
        let Some(tree_root) = tree
            .root()
            .inspect_err(|e| {
                db.report_critical(format!("Failed to parse child nodes: {e}"));
            })
            .ok()
        else {
            return children;
        };
        let mut child_nodes = tree_root.children();

        loop {
            match child_nodes.next() {
                Ok(Some(child)) => {
                    let child_offset = child.entry().offset();
                    let child_die = self.child_die(db, child_offset);
                    children.push(child_die);
                }
                Ok(None) => break,
                Err(e) => {
                    db.report_critical(format!("Failed to parse child nodes: {e}"));
                    continue;
                }
            }
        }

        children
    }

    pub fn child_die(&self, db: &'db dyn Db, offset: Offset) -> Die<'db> {
        Die::new(db, self.file(db), self.cu_offset(db), offset)
    }

    pub fn tag(&self, db: &'db dyn Db) -> gimli::DwTag {
        self.with_entry(db, |entry| entry.tag())
            .unwrap_or(gimli::DW_TAG_null)
    }

    pub fn name(&self, db: &'db dyn Db) -> Option<String> {
        self.string_attr(db, gimli::DW_AT_name)
    }

    // GROUP 3: Attribute Access (Keep - building blocks for other operations)
    pub fn get_attr(
        &self,
        db: &'db dyn Db,
        attr: gimli::DwAt,
    ) -> Option<gimli::AttributeValue<DwarfReader>> {
        self.with_entry(db, |entry| entry.attr(attr))?
            .ok()
            .flatten()
            .map(|v| v.value())
    }

    pub fn string_attr(&self, db: &'db dyn Db, attr: gimli::DwAt) -> Option<String> {
        self.with_entry_and_unit(db, |entry, unit_ref| {
            parse_die_string_attribute(entry, attr, unit_ref)
        })?
        .ok()?
    }

    pub fn print(&self, db: &'db dyn Db) -> String {
        self.with_entry_and_unit(db, |entry, unit_ref| {
            super::utils::pretty_print_die_entry(entry, unit_ref)
        })
        .unwrap_or_else(|| "entry not found".to_string())
    }

    // GROUP 5: Low-Level Access (Make private - implementation details)

    pub(super) fn with_entry_and_unit<F: FnOnce(&RawDie<'_>, &UnitRef<'_>) -> T, T>(
        &self,
        db: &'db dyn Db,
        f: F,
    ) -> Option<T> {
        let unit_ref = self.unit_ref(db)?;
        let entry = self.entry(db, &unit_ref)?;
        Some(f(&entry, &unit_ref))
    }

    pub(super) fn unit_ref(&self, db: &'db dyn Db) -> Option<UnitRef<'db>> {
        self.cu(db).unit_ref(db)
    }

    fn with_entry<F: FnOnce(&RawDie<'_>) -> T, T>(&self, db: &'db dyn Db, f: F) -> Option<T> {
        let unit_ref = self.unit_ref(db)?;
        let entry = self.entry(db, &unit_ref)?;
        Some(f(&entry))
    }

    fn entry<'a>(&self, db: &'db dyn Db, unit_ref: &'a UnitRef<'db>) -> Option<RawDie<'a>> {
        let entry = unit_ref
            .entry(self.die_offset(db))
            .inspect_err(|e| {
                db.report_critical(format!("Failed to parse entry: {e}"));
            })
            .ok()?;
        Some(entry)
    }
}

/// Get the declaration file for a DIE entry
pub fn declaration_file<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<SourceFile<'db>> {
    let decl_file_attr = entry.get_attr(db, gimli::DW_AT_decl_file);
    let Some(gimli::AttributeValue::FileIndex(file_idx)) = decl_file_attr else {
        db.report_critical(format!(
            "Failed to get decl_file attribute, got: {decl_file_attr:?}"
        ));
        return None;
    };

    let unit_ref = entry.unit_ref(db)?;

    // Get the file from the line program
    let Some(line_program) = unit_ref.line_program.clone() else {
        db.report_critical(format!("Failed to parse line program"));
        return None;
    };

    let header = line_program.header();
    let Some(file) = header.file(file_idx) else {
        db.report_critical(format!(
            "Failed to parse file index: {:#?}",
            header.file_names()
        ));
        return None;
    };

    let Some(path) = file_entry_to_path(file, &unit_ref) else {
        db.report_critical(format!("Failed to convert file entry to path"));
        return None;
    };

    Some(SourceFile::new(db, path))
}
