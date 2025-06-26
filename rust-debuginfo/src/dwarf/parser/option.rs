//! Option parser implementation using combinators

use std::sync::Arc;

use super::Parser;
use crate::{
    database::Db,
    dwarf::{
        Die,
        parser::{
            combinators::all,
            enums::{enum_discriminant, enum_named_tuple_variant},
            primitives::{attr, member_by_tag, resolve_type},
        },
    },
};
use rust_types::OptionDef;

use anyhow::Result;

pub struct OptionDefParser;

/// Parser for option types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn option_def<'db>() -> OptionDefParser {
    OptionDefParser
}

impl<'db> Parser<'db, OptionDef> for OptionDefParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<OptionDef> {
        tracing::debug!("resolving option type: {}", entry.print(db));

        // Get the variant part
        let (name, size, variants_entry) = all((
            attr::<String>(gimli::DW_AT_name),
            attr::<usize>(gimli::DW_AT_byte_size),
            member_by_tag(gimli::DW_TAG_variant_part),
        ))
        .parse(db, entry)?;

        // Parse discriminant info and variants
        let discriminant = enum_discriminant().parse(db, variants_entry)?;

        // Parse all variants
        let (_discr, ((some_offset, some_type),)) =
            enum_named_tuple_variant("Some", (resolve_type(),)).parse(db, variants_entry)?;

        Ok(OptionDef {
            name,
            some_type: Arc::new(some_type),
            some_offset,
            size,
            discriminant,
        })
    }
}
