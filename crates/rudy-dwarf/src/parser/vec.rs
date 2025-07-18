//! Vec parser implementation using combinators

use rudy_types::VecLayout;

use crate::Die;

use super::{
    children::parse_children,
    primitives::{field_path_offset, is_member_offset, resolved_generic},
    Parser,
};

/// Parser for Vec<T> type layout
pub fn vec() -> impl Parser<VecLayout<Die>> {
    parse_children((
        field_path_offset(vec!["buf", "inner", "ptr"]),
        is_member_offset("len"),
        field_path_offset(vec!["buf", "inner", "cap", "__0"]),
        resolved_generic("T"),
    ))
    .map(
        |(data_ptr_offset, length_offset, capacity_offset, inner_type)| VecLayout {
            data_ptr_offset,
            length_offset,
            capacity_offset,
            inner_type,
        },
    )
    .context("Failed to parse Vec layout")
}
