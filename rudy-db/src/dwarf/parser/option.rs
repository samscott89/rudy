//! Option parser implementation using combinators

use std::sync::Arc;

use super::Parser;
use crate::{
    database::Db,
    dwarf::{
        Die,
        parser::{
            children::child,
            combinators::all,
            enums::{PartiallyParsedEnumVariant, enum_discriminant, enum_named_tuple_variant},
            primitives::{attr, identity, member_by_tag, resolve_type_shallow},
        },
    },
};
use rudy_types::{Discriminant, OptionLayout};

use anyhow::Result;

pub struct OptionDefParser;

/// Parser for option types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn option_def() -> OptionDefParser {
    OptionDefParser
}

pub(super) fn parse_option_entry<'db>()
-> impl Parser<'db, (String, usize, Discriminant, PartiallyParsedEnumVariant<'db>)> {
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

impl<'db> Parser<'db, OptionLayout> for OptionDefParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<OptionLayout> {
        tracing::debug!("resolving option type: {}", entry.print(db));
        let (name, size, discriminant, some_variant) = parse_option_entry().parse(db, entry)?;

        // resolve the some_type
        let some_type = resolve_type_shallow().parse(db, some_variant.layout)?;

        Ok(OptionLayout {
            name,
            some_type: Arc::new(some_type),
            some_offset: some_variant.offset,
            size,
            discriminant,
        })
    }
}
