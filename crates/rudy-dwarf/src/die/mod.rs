//! Core DWARF entity types and their navigation methods

pub(crate) mod cu;
pub(crate) mod navigation;
pub(crate) mod unit;
pub(crate) mod utils;

use std::fmt;

use anyhow::Context;
pub(crate) use cu::CompilationUnitId;
use gimli::UnitSectionOffset;
pub(crate) use unit::UnitRef;
pub(crate) use utils::{file_entry_to_path, get_unit_ref_attr, parse_die_string_attribute};

use crate::{
    die::utils::pretty_print_die_entry,
    file::{
        loader::{DwarfReader, Offset, RawDie},
        DebugFile, SourceFile, SourceLocation,
    },
    DwarfDb,
};

/// References a specific DWARF debugging information entry
#[salsa::interned(debug)]
#[derive(Ord, PartialOrd)]
pub struct Die<'db> {
    pub file: DebugFile,
    pub cu_offset: UnitSectionOffset<usize>,
    pub die_offset: Offset,
}

struct DieLocation {
    path: String,
    die_offset: usize,
}

pub struct DieAccessError {
    inner: anyhow::Error,
    location: DieLocation,
}

trait ResExt {
    type V;
    fn into_die_result(self, db: &dyn DwarfDb, die: &Die) -> Result<Self::V>;
}

impl<V> ResExt for anyhow::Result<V> {
    type V = V;
    fn into_die_result(self, db: &dyn DwarfDb, die: &Die) -> Result<V> {
        self.map_err(|e| die.make_error(db, e))
    }
}

type Result<T> = std::result::Result<T, DieAccessError>;

impl fmt::Debug for DieAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for DieAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            inner,
            location: DieLocation { path, die_offset },
        } = self;
        write!(
            f,
            "Die access error at {path} {die_offset:#010x}: {inner:?}",
        )
    }
}

impl std::error::Error for DieAccessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner.source()
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        Some(self.inner.as_ref())
    }
}

impl<'db> Die<'db> {
    // GROUP 1: Core Identity (Keep - no dependencies)

    pub(crate) fn cu(&self, db: &'db dyn DwarfDb) -> CompilationUnitId<'db> {
        CompilationUnitId::new(db, self.file(db), self.cu_offset(db))
    }

    // GROUP 2: High-Cohesion Navigation + Basic Attributes (Keep - used together 90% of time)
    pub(crate) fn children(&self, db: &'db dyn DwarfDb) -> Result<Vec<Die<'db>>> {
        let mut children = vec![];

        let unit_ref = self.unit_ref(db)?;

        let mut tree = unit_ref
            .entries_tree(Some(self.die_offset(db)))
            .context("Failed to get children nodes")
            .into_die_result(db, self)?;
        let tree_root = tree
            .root()
            .context("Failed to get children nodes")
            .into_die_result(db, self)?;

        let mut child_nodes = tree_root.children();

        while let Some(child) = child_nodes
            .next()
            .context("Failed to parse child nodes")
            .into_die_result(db, self)?
        {
            let child_offset = child.entry().offset();
            let child_die = Die::new(db, self.file(db), self.cu_offset(db), child_offset);
            children.push(child_die);
        }

        Ok(children)
    }

    pub(crate) fn make_error<E: Into<anyhow::Error>>(
        &self,
        db: &'db dyn DwarfDb,
        error: E,
    ) -> DieAccessError {
        DieAccessError {
            inner: error.into(),
            location: DieLocation {
                path: self.file(db).name(db),
                die_offset: self.die_offset(db).0,
            },
        }
    }

    pub(crate) fn location(&self, db: &'db dyn DwarfDb) -> String {
        format!(
            "{} {:#010x}",
            self.file(db).name(db),
            self.cu_offset(db).as_debug_info_offset().unwrap().0 + self.die_offset(db).0,
        )
    }

    pub(crate) fn format_with_location<T: AsRef<str>>(
        &self,
        db: &'db dyn DwarfDb,
        message: T,
    ) -> String {
        format!(
            "{} for {} {:#010x}",
            message.as_ref(),
            self.file(db).name(db),
            self.die_offset(db).0,
        )
    }

    pub(crate) fn get_referenced_entry(
        &self,
        db: &'db dyn DwarfDb,
        attr: gimli::DwAt,
    ) -> Result<Die<'db>> {
        self.with_entry_and_unit(db, |entry, _| {
            get_unit_ref_attr(entry, attr)
                .map(|unit_offset| Die::new(db, self.file(db), self.cu_offset(db), unit_offset))
                .into_die_result(db, self)
        })?
    }

    pub(crate) fn tag(&self, db: &'db dyn DwarfDb) -> gimli::DwTag {
        self.with_entry(db, |entry| entry.tag())
            .unwrap_or(gimli::DW_TAG_null)
    }

    pub(crate) fn name(&self, db: &'db dyn DwarfDb) -> Result<String> {
        self.string_attr(db, gimli::DW_AT_name)
    }

    // GROUP 3: Attribute Access (Keep - building blocks for other operations)

    pub(crate) fn get_member(&self, db: &'db dyn DwarfDb, name: &str) -> Result<Die<'db>> {
        self.children(db)?
            .into_iter()
            .find(|child| child.name(db).is_ok_and(|n| n == name))
            .with_context(|| format!("Failed to find member `{name}`"))
            .into_die_result(db, self)
    }

    pub(crate) fn get_member_by_tag(
        &self,
        db: &'db dyn DwarfDb,
        tag: gimli::DwTag,
    ) -> Result<Die<'db>> {
        self.children(db)?
            .into_iter()
            .find(|child| child.tag(db) == tag)
            .with_context(|| format!("Failed to find member with tag `{tag:?}`"))
            .into_die_result(db, self)
    }

    pub(crate) fn get_udata_member_attribute(
        &self,
        db: &'db dyn DwarfDb,
        name: &str,
        attr: gimli::DwAt,
    ) -> Result<usize> {
        self.get_member(db, name)?.udata_attr(db, attr)
    }

    pub(crate) fn get_generic_type_entry(
        &self,
        db: &'db dyn DwarfDb,
        name: &str,
    ) -> Result<Die<'db>> {
        self.children(db)?
            .into_iter()
            .find(|child| {
                child.tag(db) == gimli::DW_TAG_template_type_parameter
                    && child.name(db).is_ok_and(|n| n == name)
            })
            .with_context(|| format!("Failed to find generic type entry `{name}`"))
            .into_die_result(db, self)
            .and_then(|member| member.get_referenced_entry(db, gimli::DW_AT_type))
    }

    pub(crate) fn get_attr(
        &self,
        db: &'db dyn DwarfDb,
        attr: gimli::DwAt,
    ) -> Result<gimli::AttributeValue<DwarfReader>> {
        Ok(self
            .with_entry(db, |entry| entry.attr(attr))?
            .with_context(|| format!("error fetching attribute {attr}"))
            .into_die_result(db, self)?
            .with_context(|| format!("attribute {attr} not found"))
            .into_die_result(db, self)?
            .value())
    }

    pub(crate) fn string_attr(&self, db: &'db dyn DwarfDb, attr: gimli::DwAt) -> Result<String> {
        self.with_entry_and_unit(db, |entry, unit_ref| {
            parse_die_string_attribute(entry, attr, unit_ref).into_die_result(db, self)
        })?
    }

    pub(crate) fn udata_attr(&self, db: &'db dyn DwarfDb, attr: gimli::DwAt) -> Result<usize> {
        let v = self.get_attr(db, attr)?;

        v.udata_value()
            .with_context(|| format!("attr {attr} is not a udata value, got: {v:?}"))
            .map(|v| v as usize)
            .into_die_result(db, self)
    }

    pub(crate) fn print(&self, db: &'db dyn DwarfDb) -> String {
        self.with_entry_and_unit(db, |entry, unit_ref| {
            self.format_with_location(db, pretty_print_die_entry(entry, unit_ref))
        })
        .unwrap_or_else(|e| {
            tracing::error!("Failed to print DIE entry: {e}");
            "entry not found".to_string()
        })
    }

    // GROUP 5: Low-Level Access (Make private - implementation details)

    pub(super) fn with_entry_and_unit<F: FnOnce(&RawDie<'_>, &UnitRef<'_>) -> T, T>(
        &self,
        db: &'db dyn DwarfDb,
        f: F,
    ) -> Result<T> {
        let unit_ref = self.unit_ref(db)?;
        let entry = self.entry(db, &unit_ref)?;
        Ok(f(&entry, &unit_ref))
    }

    pub(crate) fn unit_ref(&self, db: &'db dyn DwarfDb) -> Result<UnitRef<'db>> {
        self.cu(db)
            .unit_ref(db)
            .context("Failed to get unit reference")
            .into_die_result(db, self)
    }

    fn with_entry<F: FnOnce(&RawDie<'_>) -> T, T>(&self, db: &'db dyn DwarfDb, f: F) -> Result<T> {
        let unit_ref = self.unit_ref(db)?;
        let entry = self.entry(db, &unit_ref)?;
        Ok(f(&entry))
    }

    fn entry<'a>(&self, db: &'db dyn DwarfDb, unit_ref: &'a UnitRef<'db>) -> Result<RawDie<'a>> {
        unit_ref
            .entry(self.die_offset(db))
            .context("Failed to get DIE entry")
            .into_die_result(db, self)
    }
}

/// Get the position for a DIE entry
pub(crate) fn position<'db>(
    db: &'db dyn DwarfDb,
    entry: Die<'db>,
) -> Result<Option<SourceLocation<'db>>> {
    let Ok(decl_file_attr) = entry.get_attr(db, gimli::DW_AT_decl_file) else {
        return Ok(None);
    };
    let Ok(decl_line) = entry.udata_attr(db, gimli::DW_AT_decl_line) else {
        return Ok(None);
    };
    let gimli::AttributeValue::FileIndex(file_idx) = decl_file_attr else {
        return Err(entry.make_error(
            db,
            anyhow::anyhow!("Expected DW_AT_decl_file to be a FileIndex, got: {decl_file_attr:?}"),
        ));
    };

    let unit_ref = entry.unit_ref(db)?;

    // Get the file from the line program
    let Some(line_program) = unit_ref.line_program.clone() else {
        return Err(entry.make_error(db, anyhow::anyhow!("Failed to parse line program")));
    };
    let header = line_program.header();
    let Some(file) = header.file(file_idx) else {
        return Err(entry.make_error(
            db,
            anyhow::anyhow!("Failed to parse file index: {:#?}", header.file_names()),
        ));
    };

    let Some(path) = file_entry_to_path(db, file, &unit_ref) else {
        return Err(entry.make_error(db, anyhow::anyhow!("Failed to convert file entry to path")));
    };

    let source_file = SourceFile::new(db, path);
    Ok(Some(SourceLocation::new(
        db,
        source_file,
        decl_line as u64,
        None,
    )))
}
