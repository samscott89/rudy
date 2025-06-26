//! Vec parser implementation using combinators

use super::Parser;
use super::combinators::parse_children;
use super::primitives::{field_path_offset, is_member_offset, resolved_generic};
use rust_types::VecDef;

/// Parser for Vec<T> type layout
pub fn vec_parser<'db>() -> impl Parser<'db, VecDef> {
    parse_children((
        field_path_offset(vec!["buf", "inner", "ptr"]),
        is_member_offset("len"),
        resolved_generic("T"),
    ))
    .map(|(data_ptr_offset, length_offset, inner_type)| VecDef {
        data_ptr_offset,
        length_offset,
        inner_type,
    })
    .context("Failed to parse Vec layout")
}
