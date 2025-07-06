//! Option parser implementation using combinators

use anyhow::Result;
use rudy_types::{Discriminant, OptionLayout};

use super::Parser;
use crate::{
    parser::{
        children::child,
        combinators::all,
        enums::{enum_discriminant, enum_named_tuple_variant, PartiallyParsedEnumVariant},
        primitives::{attr, identity, member_by_tag, resolve_type_shallow},
    },
    Die, DwarfDb,
};

pub struct OptionDefParser;

/// Parser for option types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn option_def() -> OptionDefParser {
    OptionDefParser
}

pub(super) fn parse_option_entry(
) -> impl Parser<(String, usize, Discriminant, PartiallyParsedEnumVariant)> {
    all((
        attr::<String>(gimli::DW_AT_name),
        attr::<usize>(gimli::DW_AT_byte_size),
        member_by_tag(gimli::DW_TAG_variant_part).then(
            enum_discriminant().and(
                // gets the Some variant and eagerly pulls out the layout and offset
                // of the inner "__0" member
                child(enum_named_tuple_variant("Some", (identity(),)).map(
                    |(discriminant, ((some_offset, some_type_entry),))| {
                        PartiallyParsedEnumVariant {
                            discriminant,
                            layout: some_type_entry,
                            offset: some_offset,
                        }
                    },
                ))
                .context("expected Some variant in Option type"),
            ),
        ),
    ))
    .map(|(name, size, (discriminant, some_variant))| (name, size, discriminant, some_variant))
}

impl Parser<OptionLayout<Die>> for OptionDefParser {
    fn parse(&self, db: &dyn DwarfDb, entry: Die) -> Result<OptionLayout<Die>> {
        tracing::debug!("resolving option type: {}", entry.print(db));
        let (name, size, discriminant, some_variant) = parse_option_entry().parse(db, entry)?;

        // resolve the some_type
        let some_type = resolve_type_shallow().parse(db, some_variant.layout)?;

        Ok(OptionLayout {
            name,
            some_type,
            some_offset: some_variant.offset,
            size,
            discriminant,
        })
    }
}
