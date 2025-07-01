//! HashMap parser implementation using combinators

use super::children::parse_children;
use super::primitives::{attr, entry_type, generic, is_member, is_member_offset, member, offset};
use super::Parser;
use crate::Die;
use crate::DwarfDb;
use rudy_types::MapVariant;

type Result<T> = anyhow::Result<T>;

/// Parser for hashbrown HashMap layout
pub fn hashbrown_map<'db>() -> impl Parser<'db, MapVariant> {
    struct HashBrownMapParser;

    impl<'db> Parser<'db, MapVariant> for HashBrownMapParser {
        fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<MapVariant> {
            // table -> RawTable
            let (mut table_offset, inner_table_type) = member("table")
                .then(offset().and(entry_type()))
                .parse(db, entry)?;

            let ((pair_size, (key_offset, value_offset)), (inner_table_offset, raw_table_type)) =
                parse_children((
                    // get the T = (K, V) type so we can find the appropriate offsets in the buckets
                    generic("T").then(attr(gimli::DW_AT_byte_size).and(parse_children((
                        is_member_offset("__0"),
                        is_member_offset("__1"),
                    )))),
                    // we'll also get the RawTableInner type which contains the actual layout of the table
                    is_member("table").then(offset().and(entry_type())),
                ))
                .parse(db, inner_table_type)?;

            table_offset += inner_table_offset;

            let (bucket_mask_offset, ctrl_offset, items_offset) = parse_children((
                is_member_offset("bucket_mask"),
                is_member_offset("ctrl"),
                is_member_offset("items"),
            ))
            .parse(db, raw_table_type)?;

            // Add the table offset to the bucket_mask, ctrl, and items offsets
            let bucket_mask_offset = table_offset + bucket_mask_offset;
            let ctrl_offset = table_offset + ctrl_offset;
            let items_offset = table_offset + items_offset;

            Ok(MapVariant::HashMap {
                bucket_mask_offset,
                ctrl_offset,
                items_offset,
                pair_size,
                key_offset,
                value_offset,
            })
        }
    }

    HashBrownMapParser
}
