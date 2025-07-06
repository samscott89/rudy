//! Option parser implementation using combinators

use super::Parser;
use crate::{
    parser::primitives::{data_offset, entry_type, is_member_tag, member},
    Die,
};

/// Parser for NonNull
///
/// This basically just walks the "pointer" member to the pointer
/// type, and then from the pointer type to the inner type. Does
/// a little verification along the way.
pub fn nonnull() -> impl Parser<Die> {
    // NonNull<T> should generally be a thin wrapper around a pointer type,
    // we'll verify some of this though
    member("pointer")
        .then(
            data_offset().and(
                entry_type().then(is_member_tag(gimli::DW_TAG_pointer_type).then(entry_type())),
            ),
        )
        .map_res(|(offset, inner_type)| {
            if offset != 0 {
                anyhow::bail!("NonNull pointer offset is not zero, found: {offset:#x}");
            }

            // Return the inner type
            Ok(inner_type)
        })
}
