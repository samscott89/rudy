//! DWARF-specific primitive parsers and utilities

use std::convert::Infallible;

use anyhow::Context as _;

use super::{Parser, Result};
use crate::{
    die::utils::get_string_attr, file::DwarfReader, types::DieTypeDefinition, Die, DwarfDb,
};

/// Parser for getting offset values
pub fn offset() -> Offset {
    Offset
}

pub struct Offset;

impl<'db> Parser<'db, usize> for Offset {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<usize> {
        Ok(entry.udata_attr(db, gimli::DW_AT_data_member_location)?)
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
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<usize> {
        Ok(entry.udata_attr(db, self.attr)?)
    }
}

impl<'db> Parser<'db, u64> for Attr<u64> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<u64> {
        let value = entry.get_attr(db, self.attr)?;
        if let Some(v) = value.udata_value() {
            Ok(v)
        } else if let Some(v) = value.sdata_value() {
            if v < 0 {
                return Err(anyhow::anyhow!(
                    "Value {} is negative, cannot fit in u64",
                    v
                ));
            }
            Ok(v as u64)
        } else {
            Err(anyhow::anyhow!(
                "Expected {} to be a signed or unsigned data attribute, found: {value:?}",
                self.attr,
            ))
        }
    }
}

impl<'db> Parser<'db, i64> for Attr<i64> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<i64> {
        let value = entry.get_attr(db, self.attr)?;
        if let Some(v) = value.udata_value() {
            if v > i64::MAX as u64 {
                return Err(anyhow::anyhow!("Value {} exceeds i64 maximum", v));
            }
            Ok(v as i64)
        } else if let Some(v) = value.sdata_value() {
            Ok(v)
        } else {
            Err(anyhow::anyhow!(
                "Expected {} to be a signed or unsigned data attribute, found: {value:?}",
                self.attr,
            ))
        }
    }
}

impl<'db> Parser<'db, i128> for Attr<i128> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<i128> {
        let value = entry.get_attr(db, self.attr)?;
        if let Some(v) = value.udata_value() {
            Ok(v as i128)
        } else if let Some(v) = value.sdata_value() {
            Ok(v as i128)
        } else {
            Err(anyhow::anyhow!(
                "Expected {} to be a signed or unsigned data attribute, found: {value:?}",
                self.attr,
            ))
        }
    }
}

impl<'db> Parser<'db, String> for Attr<String> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<String> {
        Ok(entry.string_attr(db, self.attr)?)
    }
}

impl<'db> Parser<'db, Option<String>> for Attr<Option<String>> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Option<String>> {
        entry.with_entry_and_unit(db, |entry, unit_ref| {
            get_string_attr(entry, self.attr, unit_ref)
        })?
    }
}

impl<'db> Parser<'db, gimli::AttributeValue<DwarfReader>>
    for Attr<gimli::AttributeValue<DwarfReader>>
{
    fn parse(
        &self,
        db: &'db dyn DwarfDb,
        entry: Die<'db>,
    ) -> Result<gimli::AttributeValue<DwarfReader>> {
        Ok(entry.get_attr(db, self.attr)?)
    }
}

impl<'db> Parser<'db, gimli::DwLang> for Attr<gimli::DwLang> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<gimli::DwLang> {
        let value = entry.get_attr(db, self.attr)?;
        if let gimli::AttributeValue::Language(lang) = value {
            Ok(lang)
        } else {
            Err(anyhow::anyhow!(
                "Expected {} to be a language attribute, found: {value:?}",
                self.attr,
            ))
        }
    }
}

impl<'db> Parser<'db, Die<'db>> for Attr<Die<'db>> {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        Ok(entry.get_referenced_entry(db, self.attr)?)
    }
}

/// Parser that gets an optional attribute
pub struct OptionalAttr<T> {
    attr: gimli::DwAt,
    _marker: std::marker::PhantomData<T>,
}

impl<'db, T> Parser<'db, Option<T>> for OptionalAttr<T>
where
    Attr<T>: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Option<T>> {
        Ok(attr(self.attr).parse(db, entry).ok())
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
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        Ok(entry.get_member(db, &self.name)?)
    }
}

/// Check if current entry has expected name and return it
pub struct IsMember {
    pub(super) expected_name: String,
}

impl<'db> Parser<'db, Die<'db>> for IsMember {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        let entry_name = entry.name(db).context("Failed to get entry name")?;
        if entry_name == self.expected_name {
            Ok(entry)
        } else {
            Err(anyhow::anyhow!(
                "Expected field '{}', found '{entry_name}'",
                self.expected_name,
            ))
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
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<usize> {
        let entry_name = entry
            .name(db)
            .map_err(|e| crate::Error::from(anyhow::anyhow!("Failed to get entry name: {}", e)))?;
        if entry_name == self.expected_name {
            entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .with_context(|| format!("Failed to get offset for field '{}'", self.expected_name))
        } else {
            Err(anyhow::anyhow!(
                "Expected field '{}', found '{entry_name}'",
                self.expected_name,
            ))
        }
    }
}

pub fn is_member_offset(name: &str) -> IsMemberOffset {
    IsMemberOffset {
        expected_name: name.to_string(),
    }
}

/// Find a member by tag and return its Die
pub fn member_by_tag(tag: gimli::DwTag) -> MemberByTag {
    MemberByTag { tag }
}

pub struct MemberByTag {
    tag: gimli::DwTag,
}

impl<'db> Parser<'db, Die<'db>> for MemberByTag {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        entry
            .get_member_by_tag(db, self.tag)
            .with_context(|| format!("Failed to find member with tag '{}'", self.tag,))
    }
}

/// Check if current entry has expected tag and return it
pub struct IsMemberTag {
    expected_tag: gimli::DwTag,
}

impl<'db> Parser<'db, Die<'db>> for IsMemberTag {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        let entry_tag = entry.tag(db);
        if entry_tag == self.expected_tag {
            Ok(entry)
        } else {
            Err(anyhow::anyhow!(
                "Expected entry to have tag '{}', found '{entry_tag}'",
                self.expected_tag,
            ))
        }
    }
}

pub fn is_member_tag(tag: gimli::DwTag) -> IsMemberTag {
    IsMemberTag { expected_tag: tag }
}

/// Check if current entry matches expected generic name
pub struct Generic {
    expected_name: String,
}

impl<'db> Parser<'db, Die<'db>> for Generic {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        if entry.tag(db) != gimli::DW_TAG_template_type_parameter {
            return Err(anyhow::anyhow!(
                "Expected generic type parameter, found tag {}",
                entry.tag(db)
            ));
        }
        let entry_name = entry
            .name(db)
            .map_err(|e| crate::Error::from(anyhow::anyhow!("Failed to get entry name: {}", e)))?;
        if entry_name == self.expected_name {
            Ok(entry.get_referenced_entry(db, gimli::DW_AT_type)?)
        } else {
            Err(anyhow::anyhow!(
                "Expected generic '{}', found '{entry_name}'",
                self.expected_name,
            ))
        }
    }
}

pub fn generic(name: &str) -> Generic {
    Generic {
        expected_name: name.to_string(),
    }
}

pub fn resolved_generic<'db>(name: &str) -> impl Parser<'db, DieTypeDefinition<'db>> {
    generic(name).then(resolve_type_shallow())
}

/// Combinator that resolves a Die into a type definition
pub struct ResolveType;

impl<'db> Parser<'db, DieTypeDefinition<'db>> for ResolveType {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<DieTypeDefinition<'db>> {
        Ok(crate::types::resolve_type_offset(db, entry)?)
    }
}

/// Parser that does nothing, just returns the entry as is
pub fn identity() -> Identity {
    Identity
}

pub struct Identity;

impl<'db> Parser<'db, Die<'db>> for Identity {
    fn parse(&self, _db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Die<'db>> {
        Ok(entry)
    }
}

#[allow(dead_code)]
pub fn resolve_type() -> ResolveType {
    ResolveType
}

/// Combinator that resolves a Die into a type definition (shallow)
pub struct ResolveTypeShallow;

impl<'db> Parser<'db, DieTypeDefinition<'db>> for ResolveTypeShallow {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<DieTypeDefinition<'db>> {
        Ok(crate::types::shallow_resolve_type(db, entry)?)
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
    fn parse(&self, db: &'db dyn DwarfDb, mut entry: Die<'db>) -> Result<(Die<'db>, usize)> {
        let mut path_iter = self.path.iter();

        let mut offset = 0;
        // Start with the first field
        let Some(first_field) = path_iter.next() else {
            // or error?
            return Ok((entry, 0));
        };

        if &entry.name(db)? != first_field {
            return Err(anyhow::anyhow!(
                "Expected entry name '{}', found '{}'",
                first_field,
                entry.name(db)?
            ));
        }
        offset += entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .with_context(|| format!("Failed to get offset for field '{first_field}'"))?;
        entry = entry
            .get_referenced_entry(db, gimli::DW_AT_type)
            .with_context(|| format!("Failed to resolve type for field '{first_field}'"))?;
        for field_name in path_iter {
            entry = entry
                .get_member(db, field_name)
                .with_context(|| format!("Failed to navigate to field '{field_name}'"))?;
            offset += entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .with_context(|| format!("Failed to get offset for field '{field_name}'"))?;
            entry = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .with_context(|| format!("Failed to resolve type for field '{field_name}'"))?;
        }

        Ok((entry, offset))
    }
}

/// Parse a field path and return the final offset
pub fn field_path_offset<'db>(path: Vec<&str>) -> impl Parser<'db, usize> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}

pub fn rust_cu<'db>() -> impl Parser<'db, bool> {
    attr(gimli::DW_AT_language).map(|lang| matches!(lang, gimli::DW_LANG_Rust))
}

pub fn name<'db>() -> impl Parser<'db, Option<String>> {
    attr(gimli::DW_AT_name)
}

pub fn tag<'db>() -> impl Parser<'db, gimli::DwTag> {
    super::from_fn(|db, entry: Die<'_>| Ok::<_, Infallible>(entry.tag(db)))
}

pub fn die_offset<'db>() -> impl Parser<'db, usize> {
    super::from_fn(|db, entry: Die<'_>| Ok::<_, Infallible>(entry.die_offset(db).0))
}
