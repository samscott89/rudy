//! Vec parser implementation using combinators

use rudy_types::VecLayout;

use super::{
    children::parse_children,
    primitives::{field_path_offset, is_member_offset, resolved_generic},
    Parser,
};

/// Parser for Vec<T> type layout
pub fn vec<'db>() -> impl Parser<'db, VecLayout> {
    parse_children((
        field_path_offset(vec!["buf", "inner", "ptr"]),
        is_member_offset("len"),
        resolved_generic("T"),
    ))
    .map(|(data_ptr_offset, length_offset, inner_type)| VecLayout {
        data_ptr_offset,
        length_offset,
        inner_type,
    })
    .context("Failed to parse Vec layout")
}
