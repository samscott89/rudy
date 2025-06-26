//! Generic parser combinators that work with any Parser implementations

use anyhow::Context as _;

use super::{Parser, Result};
use crate::database::Db;
use crate::dwarf::Die;

/// Combinator that applies two parsers and combines their results
pub struct And<P1, P2, T, U> {
    pub(super) first: P1,
    pub(super) second: P2,
    pub(super) _marker: std::marker::PhantomData<(T, U)>,
}

impl<'db, T, U, P1, P2> Parser<'db, (T, U)> for And<P1, P2, T, U>
where
    P1: Parser<'db, T>,
    P2: Parser<'db, U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<(T, U)> {
        let first_result = self.first.parse(db, entry)?;
        let second_result = self.second.parse(db, entry)?;
        Ok((first_result, second_result))
    }
}

/// Combinator that transforms parser output
pub struct Map<P, F, T> {
    pub(super) parser: P,
    pub(super) f: F,
    pub(super) _marker: std::marker::PhantomData<T>,
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

/// Sequential combinator - applies parsers in sequence, each operating on the result of the previous
pub struct Then<P1, P2> {
    pub(super) first: P1,
    pub(super) second: P2,
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

/// Combinator that adds context to errors
pub struct Context<P> {
    pub(super) parser: P,
    pub(super) context: String,
}

impl<'db, T, P> Parser<'db, T> for Context<P>
where
    P: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<T> {
        self.parser
            .parse(db, entry)
            .with_context(|| entry.format_with_location(db, &self.context))
    }
}

