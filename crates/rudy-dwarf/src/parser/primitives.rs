//! DWARF-specific primitive parsers and utilities

use std::convert::Infallible;

use anyhow::Context as _;

use super::{Parser, Result};
use crate::{
    die::utils::get_string_attr, file::DwarfReader, types::DieTypeDefinition, Die, DwarfDb,
};

/// Parser for getting offset values
pub fn data_offset() -> DataOffset {
    DataOffset
}

pub struct DataOffset;

impl Parser<usize> for DataOffset {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<usize> {
        Ok(entry.udata_attr(db, gimli::DW_AT_data_member_location)?)
    }
}

/// Generic attribute parser
pub fn attr<T>(attr: gimli::DwAt) -> Attr<T> {
    Attr::new(attr)
}

pub fn entry_type() -> Attr<Die> {
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

impl Parser<usize> for Attr<usize> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<usize> {
        Ok(entry.udata_attr(db, self.attr)?)
    }
}

impl Parser<u64> for Attr<u64> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<u64> {
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

impl Parser<i64> for Attr<i64> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<i64> {
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

impl Parser<i128> for Attr<i128> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<i128> {
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

impl Parser<String> for Attr<String> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<String> {
        Ok(entry.string_attr(db, self.attr)?)
    }
}

impl Parser<Option<String>> for Attr<Option<String>> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Option<String>> {
        entry.with_entry_and_unit(db, |entry, unit_ref| {
            get_string_attr(entry, self.attr, unit_ref)
        })?
    }
}

impl Parser<gimli::AttributeValue<DwarfReader>> for Attr<gimli::AttributeValue<DwarfReader>> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<gimli::AttributeValue<DwarfReader>> {
        Ok(entry.get_attr(db, self.attr)?)
    }
}

impl Parser<gimli::DwLang> for Attr<gimli::DwLang> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<gimli::DwLang> {
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

impl Parser<Die> for Attr<Die> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
        Ok(entry.get_referenced_entry(db, self.attr)?)
    }
}

/// Parser that gets an optional attribute
pub struct OptionalAttr<T> {
    attr: gimli::DwAt,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Parser<Option<T>> for OptionalAttr<T>
where
    Attr<T>: Parser<T>,
{
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Option<T>> {
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

impl Parser<Die> for Member {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
        Ok(entry.get_member(db, &self.name)?)
    }
}

/// Check if current entry has expected name and return it
pub struct IsMember {
    pub(super) expected_name: String,
}

impl Parser<Die> for IsMember {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
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

impl Parser<usize> for IsMemberOffset {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<usize> {
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

impl Parser<Die> for MemberByTag {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
        entry
            .get_member_by_tag(db, self.tag)
            .with_context(|| format!("Failed to find member with tag '{}'", self.tag,))
    }
}

/// Check if current entry has expected tag and return it
pub fn is_member_tag(tag: gimli::DwTag) -> IsMemberTag {
    IsMemberTag { expected_tag: tag }
}

pub struct IsMemberTag {
    expected_tag: gimli::DwTag,
}

impl Parser<Die> for IsMemberTag {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
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

/// Check if current entry matches expected generic name
pub struct Generic {
    expected_name: String,
}

impl Parser<Die> for Generic {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<Die> {
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

pub fn resolved_generic(name: &str) -> impl Parser<DieTypeDefinition> {
    generic(name).then(resolve_type_shallow())
}

/// Combinator that resolves a Die into a type definition
pub struct ResolveType;

impl Parser<DieTypeDefinition> for ResolveType {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<DieTypeDefinition> {
        Ok(crate::types::resolve_type_offset(db, entry)?)
    }
}

/// Parser that does nothing, just returns the entry as is
pub fn identity() -> Identity {
    Identity
}

pub struct Identity;

impl Parser<Die> for Identity {
    fn parse(&self, _db: &dyn DwarfDb, entry: Die) -> Result<Die> {
        Ok(entry)
    }
}

#[allow(dead_code)]
pub fn resolve_type() -> ResolveType {
    ResolveType
}

/// Combinator that resolves a Die into a type definition (shallow)
pub struct ResolveTypeShallow;

impl Parser<DieTypeDefinition> for ResolveTypeShallow {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<DieTypeDefinition> {
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

impl Parser<(Die, usize)> for FieldPath {
    fn parse(&self, db: &dyn DwarfDb, mut entry: Die) -> Result<(Die, usize)> {
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
pub fn field_path_offset(path: Vec<&str>) -> impl Parser<usize> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}

pub fn rust_cu() -> impl Parser<bool> {
    attr(gimli::DW_AT_language).map(|lang| matches!(lang, gimli::DW_LANG_Rust))
}

pub fn name() -> impl Parser<Option<String>> {
    attr(gimli::DW_AT_name)
}

pub fn tag() -> impl Parser<gimli::DwTag> {
    super::from_fn(|db: &dyn DwarfDb, entry: Die| Ok::<_, Infallible>(entry.tag(db)))
}

pub fn offset() -> impl Parser<usize> {
    super::from_fn(|_: &dyn DwarfDb, entry: Die| Ok::<_, Infallible>(entry.offset()))
}
