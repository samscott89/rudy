//! DWARF-specific primitive parsers and utilities

use super::{Parser, Result};
use crate::database::Db;
use crate::dwarf::Die;
use rust_types::*;
use std::sync::Arc;

/// Parser for getting offset values
pub fn offset() -> Offset {
    Offset
}

pub struct Offset;

impl<'db> Parser<'db, usize> for Offset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .map_err(|e| {
                crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                    "Failed to get offset: {}",
                    e
                ))
            })
            .map(|o| o as usize)
    }
}

/// Generic attribute parser
pub fn attr<T>(attr: gimli::DwAt) -> Attr<T> {
    Attr::new(attr)
}

pub fn entry_type<'db>() -> Attr<Die<'db>> {
    Attr::new(gimli::DW_AT_type)
}

pub struct Attr<T> {
    attr: gimli::DwAt,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Attr<T> {
    pub fn new(attr: gimli::DwAt) -> Self {
        Attr {
            attr,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'db> Parser<'db, usize> for Attr<usize> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        Ok(entry.udata_attr(db, self.attr)? as usize)
    }
}

impl<'db> Parser<'db, String> for Attr<String> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<String> {
        Ok(entry.string_attr(db, self.attr)?)
    }
}

impl<'db> Parser<'db, Die<'db>> for Attr<Die<'db>> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        Ok(entry.get_referenced_entry(db, self.attr)?)
    }
}

/// Parser that gets an optional attribute
pub struct OptionalAttr<T> {
    attr: gimli::DwAt,
    _marker: std::marker::PhantomData<T>,
}

impl<'db> Parser<'db, Option<usize>> for OptionalAttr<usize> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Option<usize>> {
        Ok(entry.udata_attr(db, self.attr).ok().map(|v| v as usize))
    }
}

impl<'db> Parser<'db, Option<Die<'db>>> for OptionalAttr<Die<'db>> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Option<Die<'db>>> {
        Ok(entry.get_referenced_entry(db, self.attr).ok())
    }
}

pub fn optional_attr<T>(attr: gimli::DwAt) -> OptionalAttr<T> {
    OptionalAttr {
        attr,
        _marker: std::marker::PhantomData,
    }
}

/// Find a member by name and return its Die
pub fn member(name: &str) -> Member {
    Member {
        name: name.to_string(),
    }
}

pub struct Member {
    name: String,
}

impl<'db> Parser<'db, Die<'db>> for Member {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry.get_member(db, &self.name).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to find member '{}': {}",
                self.name,
                e
            ))
        })
    }
}

/// Check if current entry has expected name and return it
pub struct IsMember {
    expected_name: String,
}

impl<'db> Parser<'db, Die<'db>> for IsMember {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        let entry_name = entry.name(db).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to get entry name: {}",
                e
            ))
        })?;
        if entry_name == self.expected_name {
            Ok(entry)
        } else {
            Err(crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Expected field '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

pub fn is_member(name: &str) -> IsMember {
    IsMember {
        expected_name: name.to_string(),
    }
}

/// Check if current entry has expected name and get its offset
pub struct IsMemberOffset {
    expected_name: String,
}

impl<'db> Parser<'db, usize> for IsMemberOffset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        let entry_name = entry.name(db).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to get entry name: {}",
                e
            ))
        })?;
        if entry_name == self.expected_name {
            entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .map_err(|e| {
                    crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                        "Failed to get offset for field '{}': {}",
                        entry_name,
                        e
                    ))
                })
                .map(|o| o as usize)
        } else {
            Err(crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Expected field '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

pub fn is_member_offset(name: &str) -> IsMemberOffset {
    IsMemberOffset {
        expected_name: name.to_string(),
    }
}

/// Check if current entry matches expected generic name
pub struct Generic {
    expected_name: String,
}

impl<'db> Parser<'db, Die<'db>> for Generic {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        if entry.tag(db) != gimli::DW_TAG_template_type_parameter {
            return Err(crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Expected generic type parameter, found tag {:?}",
                entry.tag(db)
            )));
        }
        let entry_name = entry.name(db).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to get entry name: {}",
                e
            ))
        })?;
        if entry_name == self.expected_name {
            Ok(entry.get_referenced_entry(db, gimli::DW_AT_type)?)
        } else {
            Err(crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Expected generic '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

pub fn generic(name: &str) -> Generic {
    Generic {
        expected_name: name.to_string(),
    }
}

pub fn resolved_generic<'db>(name: &str) -> impl Parser<'db, Arc<TypeDef>> {
    generic(name).then(resolve_type()).map(Arc::new)
}

/// Parser that finds a child by tag rather than name
pub struct ChildByTag {
    tag: gimli::DwTag,
}

impl<'db> Parser<'db, Die<'db>> for ChildByTag {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry.get_member_by_tag(db, self.tag).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to find child with tag {:?}: {}",
                self.tag,
                e
            ))
        })
    }
}

pub fn child_by_tag(tag: gimli::DwTag) -> ChildByTag {
    ChildByTag { tag }
}

/// Parser that iterates through children with a specific tag
pub struct ChildrenByTag {
    tag: gimli::DwTag,
}

impl<'db> Parser<'db, Vec<Die<'db>>> for ChildrenByTag {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Vec<Die<'db>>> {
        Ok(entry
            .children(db)?
            .into_iter()
            .filter(|child| child.tag(db) == self.tag)
            .collect())
    }
}

pub fn children_by_tag(tag: gimli::DwTag) -> ChildrenByTag {
    ChildrenByTag { tag }
}

/// Combinator that resolves a Die into a type definition
pub struct ResolveType;

impl<'db> Parser<'db, TypeDef> for ResolveType {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
        crate::dwarf::resolution::shallow_resolve_type(db, entry)
    }
}

pub fn resolve_type() -> ResolveType {
    ResolveType
}

/// Combinator that resolves a Die into a type definition (shallow)
pub struct ResolveTypeShallow;

impl<'db> Parser<'db, TypeDef> for ResolveTypeShallow {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
        crate::dwarf::resolution::shallow_resolve_type(db, entry)
    }
}

pub fn resolve_type_shallow() -> ResolveTypeShallow {
    ResolveTypeShallow
}

/// A parser that follows a chain of fields, accumulating offsets
pub struct FieldPath {
    path: Vec<String>,
}

impl FieldPath {
    pub fn new(path: Vec<String>) -> Self {
        Self { path }
    }
}

impl<'db> Parser<'db, (Die<'db>, usize)> for FieldPath {
    fn parse(&self, db: &'db dyn Db, mut entry: Die<'db>) -> Result<(Die<'db>, usize)> {
        let mut path_iter = self.path.iter();

        let mut offset = 0;
        // Start with the first field
        let Some(first_field) = path_iter.next() else {
            // or error?
            return Ok((entry, 0));
        };

        if &entry.name(db)? != first_field {
            return Err(crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Expected entry name '{}', found '{}'",
                first_field,
                entry.name(db)?
            )));
        }
        offset += entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .map_err(|e| {
                crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                    "Failed to get offset for field '{}': {}",
                    first_field,
                    e
                ))
            })?;
        entry = entry
            .get_referenced_entry(db, gimli::DW_AT_type)
            .map_err(|e| {
                crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                    "Failed to resolve type for field '{first_field}': {e}",
                ))
            })?;
        while let Some(field_name) = path_iter.next() {
            entry = entry.get_member(db, field_name).map_err(|e| {
                crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                    "Failed to navigate to field '{field_name}': {e}",
                ))
            })?;
            offset += entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .map_err(|e| {
                    crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                        "Failed to get offset for field '{field_name}': {e}",
                    ))
                })? as usize;
            entry = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .map_err(|e| {
                    crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                        "Failed to resolve type for field '{field_name}': {e}",
                    ))
                })?;
        }

        Ok((entry, offset))
    }
}

/// Parse a field path and return the final offset
pub fn field_path_offset<'db>(path: Vec<&str>) -> impl Parser<'db, usize> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}
