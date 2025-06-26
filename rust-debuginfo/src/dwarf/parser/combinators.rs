//! Generic parser combinators that work with any Parser implementations

use super::{Parser, Result};
use crate::database::Db;
use crate::dwarf::Die;
use core::fmt;

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
        self.parser.parse(db, entry).map_err(|e| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!("{}: {}", self.context, e))
        })
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
            crate::dwarf::resolution::Error::from(anyhow::anyhow!("Failed to find required child"))
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
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to find required child for first parser"
            ))
        })??;
        let r2 = result2.ok_or_else(|| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
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
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to find required child for first parser"
            ))
        })??;
        let r2 = result2.ok_or_else(|| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
                "Failed to find required child for second parser"
            ))
        })??;
        let r3 = result3.ok_or_else(|| {
            crate::dwarf::resolution::Error::from(anyhow::anyhow!(
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
