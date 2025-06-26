//! Children parsing combinators and implementations

use super::{Parser, Result};
use crate::database::Db;
use crate::dwarf::Die;
use core::fmt;

/// Unified parser for tuples of parsers applied to children
pub struct ParseChildren<T> {
    parsers: T,
}

/// Main function for creating unified children parsers
pub fn parse_children<T>(parsers: T) -> ParseChildren<T> {
    ParseChildren { parsers }
}

/// Parser that applies a single parser to each child and collects results
pub struct ForEachChild<P> {
    parser: P,
}

/// Main function for creating for-each-child parser
pub fn for_each_child<P>(parser: P) -> ForEachChild<P> {
    ForEachChild { parser }
}

impl<'db, T, P> Parser<'db, Vec<T>> for ForEachChild<P>
where
    P: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Vec<T>> {
        let mut results = Vec::new();
        
        for child in entry.children(db)? {
            if let Ok(result) = self.parser.parse(db, child) {
                results.push(result);
            }
            // Note: We ignore parse failures and just collect successes
        }
        
        Ok(results)
    }
}

/// Macro to generate ParseChildren implementations for different tuple sizes
macro_rules! impl_parse_children {
    // Base case: empty tuple
    () => {
        impl<'db> Parser<'db, ()> for ParseChildren<()> {
            fn parse(&self, _db: &'db dyn Db, _entry: Die<'db>) -> Result<()> {
                Ok(())
            }
        }
    };
    
    // Single element
    ($T1:ident, $P1:ident) => {
        impl<'db, $T1, $P1> Parser<'db, ($T1,)> for ParseChildren<($P1,)>
        where
            $P1: Parser<'db, $T1>,
        {
            fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<($T1,)> {
                let mut result1: Option<Result<$T1>> = None;

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
                    anyhow::anyhow!("Failed to find required child")
                })??;
                Ok((r1,))
            }
        }
    };

    // Two elements
    ($T1:ident, $P1:ident, $T2:ident, $P2:ident) => {
        impl<'db, $T1, $T2, $P1, $P2> Parser<'db, ($T1, $T2)> for ParseChildren<($P1, $P2)>
        where
            $P1: Parser<'db, $T1>,
            $P2: Parser<'db, $T2>,
        {
            fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<($T1, $T2)> {
                let mut result1: Option<Result<$T1>> = None;
                let mut result2: Option<Result<$T2>> = None;

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
                    anyhow::anyhow!("Failed to find required child for first parser")
                })??;
                let r2 = result2.ok_or_else(|| {
                    anyhow::anyhow!("Failed to find required child for second parser")
                })??;
                Ok((r1, r2))
            }
        }
    };

    // Three elements  
    ($T1:ident, $P1:ident, $T2:ident, $P2:ident, $T3:ident, $P3:ident) => {
        impl<'db, $T1, $T2, $T3, $P1, $P2, $P3> Parser<'db, ($T1, $T2, $T3)> for ParseChildren<($P1, $P2, $P3)>
        where
            $P1: Parser<'db, $T1>,
            $P2: Parser<'db, $T2>,
            $P3: Parser<'db, $T3>,
            $T1: fmt::Debug,
            $T2: fmt::Debug,
            $T3: fmt::Debug,
        {
            fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<($T1, $T2, $T3)> {
                let mut result1: Option<Result<$T1>> = None;
                let mut result2: Option<Result<$T2>> = None;
                let mut result3: Option<Result<$T3>> = None;

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
                    anyhow::anyhow!("Failed to find required child for first parser")
                })??;
                let r2 = result2.ok_or_else(|| {
                    anyhow::anyhow!("Failed to find required child for second parser")
                })??;
                let r3 = result3.ok_or_else(|| {
                    anyhow::anyhow!("Failed to find required child for third parser")
                })??;
                Ok((r1, r2, r3))
            }
        }
    };
}

// Generate implementations for tuples of size 0 to 3
impl_parse_children!();
impl_parse_children!(T1, P1);
impl_parse_children!(T1, P1, T2, P2);
impl_parse_children!(T1, P1, T2, P2, T3, P3);