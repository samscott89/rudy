//! Parser combinator framework for DWARF type resolution
//!
//! This module provides a composable way to parse DWARF debug information
//! and extract type layout information. It aims to replace the brittle
//! manual field traversal with a more robust and reusable approach.

use crate::{
    parser::combinators::{Filter, MapWithDbAndEntry},
    Die, DwarfDb,
};

// Module structure
pub mod btreemap;
pub mod children;
pub mod combinators;
pub mod enums;
pub mod functions;
pub mod hashmap;
pub mod option;
pub mod pointers;
pub mod primitives;
pub mod result;
pub mod vec;

use combinators::{And, Context, Map, MapRes, Then};
/// Type alias for parser results
pub type Result<T> = anyhow::Result<T>;

/// Core parser trait that all combinators implement
pub trait Parser<T> {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<T>;

    /// Combine this parser with another, applying both and combining results
    fn and<U, P>(self, other: P) -> And<Self, P, T, U>
    where
        Self: Sized,
        P: Parser<U>,
    {
        And {
            first: self,
            second: other,
            _marker: std::marker::PhantomData,
        }
    }

    /// Turns the input into an optional output
    /// if the provided parser succeeds
    fn filter(self) -> Filter<Self>
    where
        Self: Sized,
    {
        Filter { parser: self }
    }

    /// Transform the output of this parser
    ///
    /// Supports both simple transformations.
    ///
    /// For more complex transformations that require access to the database or entry,
    /// use `map_with_db` or `map_with_db_and_entry`.
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

    fn map_with_entry<U, F>(self, f: F) -> MapWithDbAndEntry<Self, F, T>
    where
        Self: Sized,
        F: Fn(&dyn DwarfDb, Die, T) -> U,
    {
        MapWithDbAndEntry {
            parser: self,
            f,
            _marker: std::marker::PhantomData,
        }
    }

    /// Transform the output of this parser
    fn map_res<U, F>(self, f: F) -> MapRes<Self, F, T>
    where
        Self: Sized,
        F: Fn(T) -> Result<U>,
    {
        MapRes {
            parser: self,
            f,
            _marker: std::marker::PhantomData,
        }
    }

    /// Chain this parser with another, where the second operates on the first's result
    fn then<U, P, V>(self, next: P) -> Then<Self, P, V>
    where
        Self: Sized + Parser<V>,
        P: Parser<U>,
    {
        Then {
            first: self,
            second: next,
            _marker: std::marker::PhantomData,
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

impl<T, P> Parser<T> for &'_ P
where
    P: Parser<T>,
{
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<T> {
        <P as Parser<T>>::parse(self, db, entry)
    }
}

pub struct FromFn<F> {
    f: F,
}

pub fn from_fn<F>(f: F) -> FromFn<F> {
    FromFn { f }
}

// Functions matching the `Parser::parse` signature
// are automatically parsers
impl<T, F, E> Parser<T> for FromFn<F>
where
    F: Fn(&dyn DwarfDb, Die) -> std::result::Result<T, E>,
    E: Into<anyhow::Error>,
{
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<T> {
        (self.f)(db, entry).map_err(Into::into)
    }
}
