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

/// Combinators that transforms parser output
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

pub struct MapWithDb<P, F, T> {
    pub(super) parser: P,
    pub(super) f: F,
    pub(super) _marker: std::marker::PhantomData<T>,
}

impl<'db, T, U, P, F> Parser<'db, U> for MapWithDb<P, F, T>
where
    P: Parser<'db, T>,
    F: Fn(&'db dyn Db, T) -> U,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let result = self.parser.parse(db, entry)?;
        Ok((self.f)(db, result))
    }
}

pub struct MapWithDbAndEntry<P, F, T> {
    pub(super) parser: P,
    pub(super) f: F,
    pub(super) _marker: std::marker::PhantomData<T>,
}

impl<'db, T, U, P, F> Parser<'db, U> for MapWithDbAndEntry<P, F, T>
where
    P: Parser<'db, T>,
    F: Fn(&'db dyn Db, Die<'db>, T) -> U,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let result = self.parser.parse(db, entry)?;
        Ok((self.f)(db, entry, result))
    }
}

pub struct MapRes<P, F, T> {
    pub(super) parser: P,
    pub(super) f: F,
    pub(super) _marker: std::marker::PhantomData<T>,
}

impl<'db, T, U, P, F> Parser<'db, U> for MapRes<P, F, T>
where
    P: Parser<'db, T>,
    F: Fn(T) -> Result<U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let result = self.parser.parse(db, entry)?;
        (self.f)(result)
    }
}

/// Sequential combinator - applies parsers in sequence, each operating on the result of the previous
pub struct Then<P1, P2, T> {
    pub(super) first: P1,
    pub(super) second: P2,
    pub(super) _marker: std::marker::PhantomData<T>,
}

impl<'db, U, P1, P2> Parser<'db, U> for Then<P1, P2, Die<'db>>
where
    P1: Parser<'db, Die<'db>>,
    P2: Parser<'db, U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        let intermediate = self.first.parse(db, entry)?;
        self.second.parse(db, intermediate)
    }
}

impl<'db, U, P1, P2> Parser<'db, Option<U>> for Then<P1, P2, Option<Die<'db>>>
where
    P1: Parser<'db, Option<Die<'db>>>,
    P2: Parser<'db, U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Option<U>> {
        self.first
            .parse(db, entry)?
            .map(|intermediate| self.second.parse(db, intermediate))
            .transpose()
    }
}

impl<'db, U, P1, P2> Parser<'db, U> for Then<P1, P2, Result<Die<'db>>>
where
    P1: Parser<'db, Result<Die<'db>>>,
    P2: Parser<'db, U>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<U> {
        self.first
            .parse(db, entry)?
            .and_then(|intermediate| self.second.parse(db, intermediate))
    }
}

/// Combinator that filters the output of a parser, returning `None` if the parser fails
pub struct Filter<P> {
    pub(super) parser: P,
}

impl<'db, T, P> Parser<'db, Option<T>> for Filter<P>
where
    P: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Option<T>> {
        Ok(self.parser.parse(db, entry).ok())
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

/// Combinator that applies multiple parsers and collects their results into a tuple
pub fn all<T>(parsers: T) -> All<T> {
    All { parsers }
}

pub struct All<T> {
    pub(super) parsers: T,
}

/// Macro to dynamically generate All implementations
///
/// NOTE: it would be trivial to implement this for a bare tuple, but
/// we prefer to use an explicit struct/combinator for this, since there are
/// other methods like `parse_children` that explicitly expect a tuple of parsers,
/// and we want to avoid mistakes like `for_each_child((parser1, parser2))` which would
/// apply both parsers to the same child entry, rather than applying each parser to its own child.
macro_rules! impl_parse_all_for_tuples {
    (
        $($P:ident, $T:ident, $idx:tt),*
    ) => {
        impl<'db, $($T, $P,)*> Parser<'db, ($($T,)*)> for All<($($P,)*)>
        where
            $($P: Parser<'db, $T>),*
        {
            fn parse(&self, _db: &'db dyn Db, _entry: Die<'db>) -> anyhow::Result<($($T,)*)> {

                Ok((
                    $(
                        self.parsers.$idx.parse(_db, _entry)?,
                    )*
                ))
            }
        }
    };
}

// Generate implementations for different tuple sizes (0 to 8)
impl_parse_all_for_tuples!();
impl_parse_all_for_tuples!(P0, T0, 0);
impl_parse_all_for_tuples!(P0, T0, 0, P1, T1, 1);
impl_parse_all_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2);
impl_parse_all_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3);
impl_parse_all_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4);
impl_parse_all_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5
);
impl_parse_all_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6
);
impl_parse_all_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6, P7, T7, 7
);
