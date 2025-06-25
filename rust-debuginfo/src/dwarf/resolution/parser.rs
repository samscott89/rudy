//! Parser combinator framework for DWARF type resolution
//!
//! This module provides a composable way to parse DWARF debug information
//! and extract type layout information. It aims to replace the brittle
//! manual field traversal with a more robust and reusable approach.

use crate::database::Db;
use crate::dwarf::Die;
use anyhow::Context as AnyhowContext;
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

/// Parse a field by name and return its Die
pub struct Field {
    name: String,
}

impl<'db> Parser<'db, Die<'db>> for Field {
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

/// Parse field offset (data member location)
pub struct FieldOffset {
    field_name: String,
}

impl<'db> Parser<'db, usize> for FieldOffset {
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

/// Follow a field reference to get its type
pub struct FieldType {
    field_name: String,
}

impl<'db> Parser<'db, Die<'db>> for FieldType {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        let field = entry.get_member(db, &self.field_name).map_err(|e| {
            super::Error::from(anyhow::anyhow!(
                "Failed to find field '{}': {}",
                self.field_name,
                e
            ))
        })?;
        field.get_unit_ref(db, gimli::DW_AT_type).map_err(|e| {
            super::Error::from(anyhow::anyhow!(
                "Failed to get type for field '{}': {}",
                self.field_name,
                e
            ))
        })
    }
}

/// Resolve a generic type parameter
pub struct Generic {
    param_name: String,
}

impl<'db> Parser<'db, Die<'db>> for Generic {
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

/// Helper functions for creating parsers
pub fn field(name: &str) -> Field {
    Field {
        name: name.to_string(),
    }
}

pub fn field_offset(name: &str) -> FieldOffset {
    FieldOffset {
        field_name: name.to_string(),
    }
}

pub fn field_type(name: &str) -> FieldType {
    FieldType {
        field_name: name.to_string(),
    }
}

pub fn generic(name: &str) -> Generic {
    Generic {
        param_name: name.to_string(),
    }
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
            entry = entry.get_unit_ref(db, gimli::DW_AT_type).map_err(|e| {
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
pub fn field_path_offset<'db>(path: Vec<&str>) -> impl Parser<'db, usize> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect()).map(|(_, offset)| offset)
}

/// Parse a field path and resolve the final type
pub fn field_path_type<'db>(path: Vec<&str>) -> impl Parser<'db, TypeDef> {
    FieldPath::new(path.into_iter().map(|s| s.to_string()).collect())
        .map(|(die, _)| die)
        .then(resolve_type())
}

/// Example: Vec parser using combinators
/// This shows how the existing Vec resolver could be rewritten
pub fn vec_parser<'db>() -> impl Parser<'db, VecDef> {
    // Parse the data pointer offset by following: buf -> inner -> ptr
    let data_ptr_parser = field_path_offset(vec!["buf", "inner", "ptr"])
        .context("Failed to resolve Vec data pointer path");

    // Parse the length offset directly
    let length_parser = field_offset("len").context("Failed to resolve Vec length offset");

    // Parse the inner type from generic parameter T
    let inner_type_parser = generic("T")
        .then(resolve_type())
        .map(Arc::new)
        .context("Failed to resolve Vec inner type");

    // Combine all three parsers to build VecDef
    data_ptr_parser
        .and(length_parser, |data_ptr_offset, length_offset| {
            Ok((data_ptr_offset, length_offset))
        })
        .and(inner_type_parser, |offsets, inner_type| {
            let (data_ptr_offset, length_offset) = offsets;
            Ok(VecDef {
                data_ptr_offset,
                length_offset,
                inner_type,
            })
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
        // This test just ensures our API design compiles
        let _field_parser = field("buf");
        let _offset_parser = field_offset("len");
        let _path_parser = path!["buf", "inner", "ptr"];

        // Test that combinators chain properly
        let _combined = field("buf")
            .map(|_die| 42usize)
            .context("Failed to parse buf field");

        // Test that the Vec parser compiles
        let _vec_parser = vec_parser();
    }
}
