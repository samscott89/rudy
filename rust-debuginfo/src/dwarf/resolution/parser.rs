//! Parser combinator framework for DWARF type resolution
//!
//! This module provides a composable way to parse DWARF debug information
//! and extract type layout information. It aims to replace the brittle
//! manual field traversal with a more robust and reusable approach.

use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::resolution::resolve_entry_type_shallow;
use core::fmt;
use rust_types::*;
use std::sync::Arc;

type Result<T> = std::result::Result<T, super::Error>;

/// Core parser trait that all combinators implement
pub trait Parser<'db, T> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<T>;

    /// Combine this parser with another, applying both and combining results
    fn and<U, V, P, F>(self, other: P, combine: F) -> And<Self, P, F, T, U>
    where
        Self: Sized,
        P: Parser<'db, U>,
        F: Fn(T, U) -> Result<V>,
    {
        And {
            first: self,
            second: other,
            combine,
            _marker: std::marker::PhantomData,
        }
    }

    /// Transform the output of this parser
    fn map<U, F>(self, f: F) -> Map<Self, F, T>
    where
        Self: Sized,
        F: Fn(T) -> U,
    {
        Map {
            parser: self,
            f,
            _marker: std::marker::PhantomData,
        }
    }

    /// Chain this parser with another, where the second operates on the first's result
    fn then<U, P>(self, next: P) -> Then<Self, P>
    where
        Self: Sized + Parser<'db, Die<'db>>,
        P: Parser<'db, U>,
    {
        Then {
            first: self,
            second: next,
        }
    }

    /// Add context to errors from this parser
    fn context<S: Into<String>>(self, ctx: S) -> Context<Self>
    where
        Self: Sized,
    {
        Context {
            parser: self,
            context: ctx.into(),
        }
    }
}

pub fn offset() -> Offset {
    Offset
}

pub struct Offset;

impl<'db> Parser<'db, usize> for Offset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get offset: {}", e)))
            .map(|o| o as usize)
    }
}

/// Find a child member by name and return its Die
pub struct ChildField {
    name: String,
}

impl<'db> Parser<'db, Die<'db>> for ChildField {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry.get_member(db, &self.name).map_err(|e| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find field '{}': {}",
                self.name,
                e
            ))
        })
    }
}

pub fn attr<T>(attr: gimli::DwAt) -> Attr<T> {
    Attr::new(attr)
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
        Ok(entry.udata_attr(db, self.attr)?)
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

/// Check if current entry has expected name and return it
pub struct Field {
    expected_name: String,
}

impl<'db> Parser<'db, Die<'db>> for Field {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        let entry_name = entry
            .name(db)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get entry name: {}", e)))?;
        if entry_name == self.expected_name {
            Ok(entry)
        } else {
            Err(super::Error::from(anyhow::anyhow!(
                "Expected field '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

/// Find child member by name and get its offset
pub struct ChildFieldOffset {
    field_name: String,
}

impl<'db> Parser<'db, usize> for ChildFieldOffset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        entry
            .get_udata_member_attribute(db, &self.field_name, gimli::DW_AT_data_member_location)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to get offset for field '{}': {}",
                    self.field_name,
                    e
                ))
            })
            .map(|offset| offset as usize)
    }
}

/// Check if current entry has expected name and get its offset
pub struct FieldOffset {
    expected_name: String,
}

impl<'db> Parser<'db, usize> for FieldOffset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        let entry_name = entry
            .name(db)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get entry name: {}", e)))?;
        if entry_name == self.expected_name {
            entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .map_err(|e| {
                    super::Error::from(anyhow::anyhow!(
                        "Failed to get offset for field '{}': {}",
                        entry_name,
                        e
                    ))
                })
                .map(|o| o as usize)
        } else {
            Err(super::Error::from(anyhow::anyhow!(
                "Expected field '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

/// Find child field by name and get its type
pub struct ChildFieldType {
    field_name: String,
}

impl<'db> Parser<'db, Die<'db>> for ChildFieldType {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        let field = entry.get_member(db, &self.field_name).map_err(|e| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find field '{}': {}",
                self.field_name,
                e
            ))
        })?;
        field
            .get_referenced_entry(db, gimli::DW_AT_type)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to get type for field '{}': {}",
                    self.field_name,
                    e
                ))
            })
    }
}

/// Find generic type parameter from parent entry
pub struct ChildGeneric {
    param_name: String,
}

impl<'db> Parser<'db, Die<'db>> for ChildGeneric {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry
            .get_generic_type_entry(db, &self.param_name)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to resolve generic parameter '{}': {}",
                    self.param_name,
                    e
                ))
            })
    }
}

/// Check if current entry matches expected generic name
pub struct Generic {
    expected_name: String,
}

impl<'db> Parser<'db, Arc<TypeDef>> for Generic {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Arc<TypeDef>> {
        if entry.tag(db) != gimli::DW_TAG_template_type_parameter {
            return Err(super::Error::from(anyhow::anyhow!(
                "Expected generic type parameter, found tag {:?}",
                entry.tag(db)
            )));
        }
        let entry_name = entry
            .name(db)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get entry name: {}", e)))?;
        if entry_name == self.expected_name {
            Ok(Arc::new(resolve_entry_type_shallow(db, entry)?))
        } else {
            Err(super::Error::from(anyhow::anyhow!(
                "Expected generic '{}', found '{}'",
                self.expected_name,
                entry_name
            )))
        }
    }
}

/// Combinator that applies two parsers and combines their results
pub struct And<P1, P2, F, T, U> {
    first: P1,
    second: P2,
    combine: F,
    _marker: std::marker::PhantomData<(T, U)>,
}

impl<'db, T, U, V, P1, P2, F> Parser<'db, V> for And<P1, P2, F, T, U>
where
    P1: Parser<'db, T>,
    P2: Parser<'db, U>,
    F: Fn(T, U) -> Result<V>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<V> {
        let first_result = self.first.parse(db, entry)?;
        let second_result = self.second.parse(db, entry)?;
        (self.combine)(first_result, second_result)
    }
}

/// Combinator that transforms parser output
pub struct Map<P, F, T> {
    parser: P,
    f: F,
    _marker: std::marker::PhantomData<T>,
}

impl<'db, T, U, P, F> Parser<'db, U> for Map<P, F, T>
where
    P: Parser<'db, T>,
    F: Fn(T) -> U,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let result = self.parser.parse(db, entry)?;
        Ok((self.f)(result))
    }
}

/// Combinator that adds context to errors
pub struct Context<P> {
    parser: P,
    context: String,
}

impl<'db, T, P> Parser<'db, T> for Context<P>
where
    P: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<T> {
        self.parser
            .parse(db, entry)
            .map_err(|e| super::Error::from(anyhow::anyhow!("{}: {}", self.context, e)))
    }
}

/// Helper functions for creating child search parsers
pub fn child_field(name: &str) -> ChildField {
    ChildField {
        name: name.to_string(),
    }
}

pub fn child_field_offset(name: &str) -> ChildFieldOffset {
    ChildFieldOffset {
        field_name: name.to_string(),
    }
}

pub fn child_field_type(name: &str) -> ChildFieldType {
    ChildFieldType {
        field_name: name.to_string(),
    }
}

pub fn child_generic(name: &str) -> ChildGeneric {
    ChildGeneric {
        param_name: name.to_string(),
    }
}

/// Helper functions for creating name-matching parsers
pub fn field(name: &str) -> Field {
    Field {
        expected_name: name.to_string(),
    }
}

pub fn field_offset(name: &str) -> FieldOffset {
    FieldOffset {
        expected_name: name.to_string(),
    }
}

pub fn generic(name: &str) -> Generic {
    Generic {
        expected_name: name.to_string(),
    }
}

/// Unified parser for tuples of parsers applied to children
pub struct ParseChildren<T> {
    parsers: T,
}

/// Implement for tuples of different sizes
impl<'db, T1, P1> Parser<'db, (T1,)> for ParseChildren<(P1,)>
where
    P1: Parser<'db, T1>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<(T1,)> {
        let mut result1: Option<Result<T1>> = None;

        for child in entry.children(db)? {
            if result1.is_none() {
                match self.parsers.0.parse(db, child) {
                    Ok(res) => result1 = Some(Ok(res)),
                    Err(_) => {} // Try next child
                }
            }

            // Early exit if all found
            if result1.is_some() {
                break;
            }
        }

        let r1 = result1.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!("Failed to find required child"))
        })??;
        Ok((r1,))
    }
}

impl<'db, T1, T2, P1, P2> Parser<'db, (T1, T2)> for ParseChildren<(P1, P2)>
where
    P1: Parser<'db, T1>,
    P2: Parser<'db, T2>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<(T1, T2)> {
        let mut result1: Option<Result<T1>> = None;
        let mut result2: Option<Result<T2>> = None;

        for child in entry.children(db)? {
            if result1.is_none() {
                match self.parsers.0.parse(db, child) {
                    Ok(res) => result1 = Some(Ok(res)),
                    Err(_) => {} // Try next child
                }
            }
            if result2.is_none() {
                match self.parsers.1.parse(db, child) {
                    Ok(res) => result2 = Some(Ok(res)),
                    Err(_) => {} // Try next child
                }
            }

            // Early exit if all found
            if result1.is_some() && result2.is_some() {
                break;
            }
        }

        let r1 = result1.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find required child for first parser"
            ))
        })??;
        let r2 = result2.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find required child for second parser"
            ))
        })??;
        Ok((r1, r2))
    }
}

impl<'db, T1, T2, T3, P1, P2, P3> Parser<'db, (T1, T2, T3)> for ParseChildren<(P1, P2, P3)>
where
    P1: Parser<'db, T1>,
    P2: Parser<'db, T2>,
    P3: Parser<'db, T3>,
    T1: fmt::Debug,
    T2: fmt::Debug,
    T3: fmt::Debug,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<(T1, T2, T3)> {
        let mut result1: Option<Result<T1>> = None;
        let mut result2: Option<Result<T2>> = None;
        let mut result3: Option<Result<T3>> = None;

        for child in entry.children(db)? {
            tracing::debug!(
                "Parsing child: {}",
                child.format_with_location(db, child.print(db))
            );
            if result1.is_none() {
                match self.parsers.0.parse(db, child) {
                    Ok(res) => {
                        tracing::debug!("Found result1: {:?}", res);
                        result1 = Some(Ok(res))
                    }
                    Err(_) => {} // Try next child
                }
            }
            if result2.is_none() {
                match self.parsers.1.parse(db, child) {
                    Ok(res) => {
                        tracing::debug!("Found result2: {:?}", res);
                        result2 = Some(Ok(res))
                    }
                    Err(_) => {} // Try next child
                }
            }
            if result3.is_none() {
                match self.parsers.2.parse(db, child) {
                    Ok(res) => {
                        tracing::debug!("Found result3: {:?}", res);
                        result3 = Some(Ok(res))
                    }
                    Err(_) => {} // Try next child
                }
            }

            // Early exit if all found
            if result1.is_some() && result2.is_some() && result3.is_some() {
                break;
            }
        }

        let r1 = result1.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find required child for first parser"
            ))
        })??;
        let r2 = result2.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find required child for second parser"
            ))
        })??;
        let r3 = result3.ok_or_else(|| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find required child for third parser"
            ))
        })??;
        Ok((r1, r2, r3))
    }
}

/// Main function for creating unified children parsers
pub fn parse_children<T>(parsers: T) -> ParseChildren<T> {
    ParseChildren { parsers }
}

/// Utility for accumulating offsets along a field path
pub struct OffsetAccumulator {
    current_offset: usize,
}

impl OffsetAccumulator {
    pub fn new() -> Self {
        Self { current_offset: 0 }
    }

    pub fn add_field_offset<'db>(
        &mut self,
        db: &'db dyn Db,
        entry: Die<'db>,
        field_name: &str,
    ) -> Result<()> {
        let offset = entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to get offset for field '{}': {}",
                    field_name,
                    e
                ))
            })?;
        self.current_offset += offset as usize;
        Ok(())
    }

    pub fn get_offset(&self) -> usize {
        self.current_offset
    }
}

/// A parser that follows a chain of fields, accumulating offsets
pub struct ChildFieldPath {
    path: Vec<String>,
}

impl ChildFieldPath {
    pub fn new(path: Vec<String>) -> Self {
        Self { path }
    }
}

impl<'db> Parser<'db, (Die<'db>, usize)> for ChildFieldPath {
    fn parse(&self, db: &'db dyn Db, mut entry: Die<'db>) -> Result<(Die<'db>, usize)> {
        let mut accumulator = OffsetAccumulator::new();

        for field_name in &self.path {
            entry = entry.get_member(db, field_name).map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to navigate to field '{}': {}",
                    field_name,
                    e
                ))
            })?;
            accumulator.add_field_offset(db, entry, field_name)?;
            entry = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .map_err(|e| {
                    super::Error::from(anyhow::anyhow!(
                        "Failed to resolve type for field '{}': {}",
                        field_name,
                        e
                    ))
                })?;
        }

        Ok((entry, accumulator.get_offset()))
    }
}

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
            return Err(super::Error::from(anyhow::anyhow!(
                "Expected entry name '{}', found '{}'",
                first_field,
                entry.name(db)?
            )));
        }
        offset += entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to get offset for field '{}': {}",
                    first_field,
                    e
                ))
            })?;
        entry = entry
            .get_referenced_entry(db, gimli::DW_AT_type)
            .map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to resolve type for field '{first_field}': {e}",
                ))
            })?;
        while let Some(field_name) = path_iter.next() {
            entry = entry.get_member(db, field_name).map_err(|e| {
                super::Error::from(anyhow::anyhow!(
                    "Failed to navigate to field '{field_name}': {e}",
                ))
            })?;
            offset += entry
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .map_err(|e| {
                    super::Error::from(anyhow::anyhow!(
                        "Failed to get offset for field '{field_name}': {e}",
                    ))
                })? as usize;
            entry = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .map_err(|e| {
                    super::Error::from(anyhow::anyhow!(
                        "Failed to resolve type for field '{field_name}': {e}",
                    ))
                })?;
        }

        Ok((entry, offset))
    }
}

/// Convenience macro for creating field paths
#[macro_export]
macro_rules! path {
    ($($field:expr),+ $(,)?) => {
        FieldPath::new(vec![$($field.to_string()),+])
    };
}

/// Sequential combinator - applies parsers in sequence, each operating on the result of the previous
pub struct Then<P1, P2> {
    first: P1,
    second: P2,
}

impl<'db, U, P1, P2> Parser<'db, U> for Then<P1, P2>
where
    P1: Parser<'db, Die<'db>>,
    P2: Parser<'db, U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let intermediate = self.first.parse(db, entry)?;
        self.second.parse(db, intermediate)
    }
}

/// Combinator that resolves a Die into a type definition
pub struct ResolveType;

impl<'db> Parser<'db, TypeDef> for ResolveType {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
        super::shallow_resolve_type(db, entry)
    }
}

pub fn resolve_type() -> ResolveType {
    ResolveType
}

/// Higher-level combinators for common patterns

/// Parse a field path and return the final offset
pub fn child_field_path_offset<'db>(path: Vec<&str>) -> impl Parser<'db, usize> {
    ChildFieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}

pub fn field_path_offset<'db>(path: Vec<&str>) -> impl Parser<'db, usize> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}

pub fn vec_parser<'db>() -> impl Parser<'db, VecDef> {
    parse_children((
        field_path_offset(vec!["buf", "inner", "ptr"]),
        field_offset("len"),
        generic("T"),
    ))
    .map(|(data_ptr_offset, length_offset, inner_type)| VecDef {
        data_ptr_offset,
        length_offset,
        inner_type,
    })
    .context("Failed to parse Vec layout")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic smoke tests to ensure the API compiles and makes sense
    // Real tests would require setting up DWARF data

    #[test]
    fn test_parser_api_compiles() {
        // Test basic parsers
        let _child_field_parser = child_field("buf");
        let _child_offset_parser = child_field_offset("len");
        let _path_parser = path!["buf", "inner", "ptr"];

        // Test that combinators chain properly
        let _combined = child_field("buf")
            .map(|_die| 42usize)
            .context("Failed to parse buf field");

        // Test that the Vec parser compiles
        let _vec_parser = vec_parser();

        // Test parse_children API
        let _single_child = parse_children((field("buf"),));
        let _dual_children = parse_children((field("buf"), field_offset("len")));
        let _triple_children = parse_children((field("buf"), field_offset("len"), generic("T")));
    }

    #[test]
    fn test_parse_children_usage_example() {
        // Example of how parse_children would be used in practice:
        //
        // Instead of this brittle approach:
        // let buf_field = entry.get_member(db, "buf")?;
        // let len_offset = entry.get_udata_member_attribute(db, "len", gimli::DW_AT_data_member_location)?;
        // let inner_type = entry.get_generic_type_entry(db, "T")?;
        //
        // You could use this declarative approach:
        // let (buf_field, len_offset, inner_type) = parse_children((
        //     field("buf"),
        //     field_offset("len"),
        //     generic("T")
        // )).parse(db, entry)?;

        // This compiles and shows the expected API
        let _parser = parse_children((field("buf"), field_offset("len"), generic("T")));
    }
}
