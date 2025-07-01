//! Option parser implementation using combinators

use std::sync::Arc;

use super::Parser;
use crate::{
    parser::{
        children::parse_children,
        combinators::all,
        enums::{enum_discriminant, enum_named_tuple_variant},
        primitives::{attr, member_by_tag, resolve_type_shallow},
    },
    Die, DwarfDb,
};
use rudy_types::ResultLayout;

use anyhow::Result;

pub struct ResultDefParser;

/// Parser for result types
///
/// We'll parse it as a generic enum, and extract out the expect "Ok" and "Err" variants
pub fn result_def() -> ResultDefParser {
    ResultDefParser
}

impl<'db> Parser<'db, ResultLayout> for ResultDefParser {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<ResultLayout> {
        tracing::debug!("resolving result type: {}", entry.print(db));

        // Get the variant part
        let (name, size, (discriminant, (ok, err))) = all((
            attr::<String>(gimli::DW_AT_name),
            attr::<usize>(gimli::DW_AT_byte_size),
            member_by_tag(gimli::DW_TAG_variant_part).then(enum_discriminant().and(
                parse_children((
                    enum_named_tuple_variant("Ok", (resolve_type_shallow(),)).map(
                        |(discriminant, ((ok_offset, ok_type),))| {
                            (discriminant, (ok_offset, ok_type))
                        },
                    ),
                    enum_named_tuple_variant("Err", (resolve_type_shallow(),)).map(
                        |(discriminant, ((err_offset, err_type),))| {
                            (discriminant, (err_offset, err_type))
                        },
                    ),
                )),
            )),
        ))
        .parse(db, entry)?;

        let (_, (ok_offset, ok_layout)) = ok;
        let (_, (err_offset, err_layout)) = err;

        Ok(ResultLayout {
            name,
            ok_type: Arc::new(ok_layout),
            ok_offset,
            err_type: Arc::new(err_layout),
            err_offset,
            size,
            discriminant,
        })
    }
}
