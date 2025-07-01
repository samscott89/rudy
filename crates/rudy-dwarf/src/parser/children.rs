//! Children parsing combinators and implementations

use super::{Parser, Result};
use crate::parser::combinators::Map;
use crate::Die;
use crate::DwarfDb;

/// Unified parser for tuples of parsers applied to children
pub struct ParseChildren<T> {
    parsers: T,
}

/// Main function for creating unified children parsers
pub fn parse_children<T>(parsers: T) -> ParseChildren<T> {
    ParseChildren { parsers }
}

fn extract_tuple<T>(tuple: (T,)) -> T {
    tuple.0
}

type ExtractTuple<T> = fn((T,)) -> T;

/// Finds the first child that matches the given parser
pub fn child<'db, P, T>(parser: P) -> Map<ParseChildren<(P,)>, ExtractTuple<T>, (T,)>
where
    P: Parser<'db, T>,
{
    Map {
        parser: parse_children((parser,)),
        f: extract_tuple,
        _marker: std::marker::PhantomData,
    }
}

/// Parser that applies a single parser to each child and collects results
pub struct ForEachChild<P> {
    parser: P,
}

pub fn for_each_child<P>(parser: P) -> ForEachChild<P> {
    ForEachChild { parser }
}

impl<'db, T, P> Parser<'db, Vec<T>> for ForEachChild<P>
where
    P: Parser<'db, T>,
{
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Vec<T>> {
        let mut results = Vec::new();

        for child in entry.children(db)? {
            match self.parser.parse(db, child) {
                Ok(result) => {
                    results.push(result);
                }
                Err(e) => {
                    tracing::trace!(
                        "Failed to parse child: {}: {}",
                        child.format_with_location(db, child.print(db)),
                        e
                    );
                    // Optionally, you could collect errors if needed
                    // results.push(Err(e));
                    // Or just log them and continue
                    // tracing::error!("Error parsing child: {}", e);
                    continue;
                }
            }
            // Note: We ignore parse failures and just collect successes
        }

        Ok(results)
    }
}
pub struct TryForEachChild<P> {
    parser: P,
}

pub fn try_for_each_child<P>(parser: P) -> TryForEachChild<P> {
    TryForEachChild { parser }
}

impl<'db, T, P> Parser<'db, Vec<T>> for TryForEachChild<P>
where
    P: Parser<'db, Option<T>>,
{
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<Vec<T>> {
        let mut results = Vec::new();

        for child in entry.children(db)? {
            if let Some(result) = self.parser.parse(db, child)? {
                results.push(result);
            }
            // Note: We ignore parse failures and just collect successes
        }

        Ok(results)
    }
}

/// Macro to dynamically generate ParseChildren implementations
macro_rules! impl_parse_children_for_tuples {
    (
        $($P:ident, $T:ident, $idx:tt),*
    ) => {
        impl<'db, $($T, $P,)*> Parser<'db, ($($T,)*)> for ParseChildren<($($P,)*)>
        where
            $($P: Parser<'db, $T>),*
        {
            fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> anyhow::Result<($($T,)*)> {

                #[allow(unused)] // unused when len=0
                let mut result = ($(
                    None::<anyhow::Result<$T>>,
                )*);

                for child in entry.children(db)? {
                    tracing::trace!(
                        "Parsing child: {}",
                        child.format_with_location(db, child.print(db))
                    );

                    $(
                        if result.$idx.is_none() {
                            match self.parsers.$idx.parse(db, child) {
                                Ok(res) => {
                                    result.$idx = Some(Ok(res));
                                }
                                Err(_) => {}
                            }
                        }
                    )*
                }

                let result = ($(
                    result.$idx.ok_or_else(|| anyhow::anyhow!("Failed to find required child for parser at index {}", $idx))??,
                )*);
                Ok(result)
            }
        }
    };
}

// Generate implementations for different tuple sizes (0 to 8)
impl_parse_children_for_tuples!();
impl_parse_children_for_tuples!(P0, T0, 0);
impl_parse_children_for_tuples!(P0, T0, 0, P1, T1, 1);
impl_parse_children_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2);
impl_parse_children_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3);
impl_parse_children_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4);
impl_parse_children_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5);
impl_parse_children_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6
);
impl_parse_children_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6, P7, T7, 7
);
