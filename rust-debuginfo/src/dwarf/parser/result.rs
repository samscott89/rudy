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
            primitives::{attr, entry_type, member_by_tag, offset, resolve_type},
        },
    },
};
use rust_types::ResultDef;

use anyhow::Result;

pub struct ResultDefParser;

/// Parser for result types
///
/// We'll parse it as a generic enum, and extract out the expect "Ok" and "Err" variants
pub fn result_def<'db>() -> ResultDefParser {
    ResultDefParser
}

impl<'db> Parser<'db, ResultDef> for ResultDefParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<ResultDef> {
        tracing::debug!("resolving result type: {}", entry.print(db));

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
        let (_discr, ((ok_offset, ok_type),)) =
            enum_named_tuple_variant("Ok", (resolve_type(),)).parse(db, variants_entry)?;
        let (_discr, ((err_offset, err_type),)) =
            enum_named_tuple_variant("Err", (resolve_type(),)).parse(db, variants_entry)?;

        Ok(ResultDef {
            name,
            ok_type: Arc::new(ok_type),
            err_type: Arc::new(err_type),
            size,
            discriminant,
        })
    }
}
