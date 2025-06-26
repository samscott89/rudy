//! BTreeMap parser implementation using combinators

use super::Parser;
use super::{
    children::for_each_child,
    primitives::{
        attr, entry_type, is_member_tag, member_by_tag, optional_attr, resolve_type_shallow,
    },
};
use crate::database::Db;
use crate::dwarf::parser::primitives::{member, resolved_generic};
use crate::dwarf::parser::{combinators::all, primitives::offset};
use crate::dwarf::{Die, resolution::resolve_entry_type};
use rust_types::{Discriminant, DiscriminantType, EnumDef, EnumVariant, MapDef, MapVariant};
use std::sync::Arc;

use anyhow::Result;

/// Parser for btree BTreeMap layout
pub fn btree_map<'db>() -> impl Parser<'db, MapDef> {
    // 0x00000ffc:           DW_TAG_structure_type
    //                         DW_AT_name      ("BTreeMap<alloc::string::String, i32, alloc::alloc::Global>")

    // 0x00001004:             DW_TAG_template_type_parameter
    //                           DW_AT_type    (0x0000241b "alloc::string::String")
    //                           DW_AT_name    ("K")

    // 0x0000100d:             DW_TAG_template_type_parameter
    //                           DW_AT_type    (0x000025cf "i32")
    //                           DW_AT_name    ("V")

    // 0x00001016:             DW_TAG_template_type_parameter
    //                           DW_AT_type    (0x000024d5 "alloc::alloc::Global")
    //                           DW_AT_name    ("A")

    // 0x0000101f:             DW_TAG_member
    //                           DW_AT_name    ("root")
    //                           DW_AT_type    (0x000005e4 "core::option::Option<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Owned, alloc::string::String, i32, alloc::collections::btree::node::marker::LeafOrInternal>>")
    //                           DW_AT_data_member_location    (0x00)

    // 0x0000102b:             DW_TAG_member
    //                           DW_AT_name    ("length")
    //                           DW_AT_type    (0x000025c8 "usize")
    //                           DW_AT_data_member_location    (0x10)

    all((
        resolved_generic("K"),
        resolved_generic("V"),
        member("root").then(offset().and(entry_type().then(root_node_type()))),
        member("length").then(offset()),
    ))
    .map(
        |(key_type, value_type, (root_offset, _), length_offset)| MapDef {
            key_type,
            value_type,
            variant: MapVariant::BTreeMap {
                length_offset,
                root_offset,
                root_layout: todo!(),
            },
        },
    )
}

pub fn root_node_type<'db>() -> impl Parser<'db, Arc<Die<'db>>> {
    is_member_tag(gimli::DW_TAG_structure_type).map(|_| todo!())
}
