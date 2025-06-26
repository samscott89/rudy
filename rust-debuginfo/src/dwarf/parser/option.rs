//! Option parser implementation using combinators

use std::sync::Arc;

use super::Parser;
use crate::{
    database::Db,
    dwarf::{
        Die,
        parser::{
            combinators::all,
            enums::{
                PartiallyParsedEnumVariant, enum_discriminant, enum_named_tuple_variant,
                named_enum_variant,
            },
            primitives::{attr, entry_type, member_by_tag, offset, resolve_type},
        },
    },
};
use rust_types::{Discriminant, OptionDef};

use anyhow::Result;

pub struct OptionDefParser;

/// Parser for option types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn option_def<'db>() -> OptionDefParser {
    OptionDefParser
}

pub(super) fn parse_option_entry<'db>()
-> impl Parser<'db, (String, usize, Discriminant, PartiallyParsedEnumVariant<'db>)> {
    all((
        attr::<String>(gimli::DW_AT_name),
        attr::<usize>(gimli::DW_AT_byte_size),
        enum_discriminant(),
        member_by_tag(gimli::DW_TAG_variant_part).then(named_enum_variant("Some")),
    ))
}

impl<'db> Parser<'db, OptionDef> for OptionDefParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<OptionDef> {
        tracing::debug!("resolving option type: {}", entry.print(db));
        let (name, size, discriminant, some_variant) = parse_option_entry().parse(db, entry)?;

        // resolve the some_type
        let (some_offset, some_type) = offset()
            .and(entry_type().then(resolve_type()))
            .parse(db, some_variant.layout)?;

        Ok(OptionDef {
            name,
            some_type: Arc::new(some_type),
            some_offset,
            size,
            discriminant,
        })
    }
}
