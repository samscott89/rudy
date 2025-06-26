//! BTreeMap parser implementation using combinators

use super::Parser;
use super::{
    children::for_each_child,
    primitives::{
        attr, entry_type, is_member_tag, member_by_tag, optional_attr, resolve_type_shallow,
    },
};
use crate::database::Db;
use crate::dwarf::parser::children::parse_children;
use crate::dwarf::parser::enums::PartiallyParsedEnumVariant;
use crate::dwarf::parser::option::parse_option_entry;
use crate::dwarf::parser::primitives::{member, resolved_generic};
use crate::dwarf::parser::{combinators::all, primitives::offset};
use crate::dwarf::{Die, resolution::resolve_entry_type};
use rust_types::{Discriminant, DiscriminantType, EnumDef, EnumVariant, MapDef, MapVariant};
use std::sync::Arc;

use anyhow::Result;

/// Parser for btree BTreeMap layout
pub fn btree_map<'db>() -> BTreeMapParser {
    BTreeMapParser
}

struct BTreeMapParser;

impl<'db> Parser<'db, MapDef> for BTreeMapParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<MapDef> {
        tracing::debug!("resolving btree map type: {}", entry.print(db));

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
        let (key_type, value_type, (root_offset, root_node_entry), length_offset) =
            all((
                resolved_generic("K"),
                resolved_generic("V"),
                member("root").then(offset().and(
                    entry_type().then(
                        is_member_tag(gimli::DW_TAG_structure_type).then(parse_option_entry()),
                    ),
                )),
                member("length").then(offset()),
            ))
            .parse(db, entry)?;

        let (name, _size, _discriminant, some_variant) = root_node_entry;
        debug_assert!(
            name.starts_with("core::option::Option<alloc::collections::btree::node::NodeRef<"),
            "expected noderef, got: {name}"
        );

        let node_ref_type = some_variant.layout;

        // next, we have the NodeRef struct to parse

        // 0x000020ee:           DW_TAG_structure_type
        //                         DW_AT_name      ("NodeRef<alloc::collections::btree::node::marker::Owned, alloc::string::String, i32,
        //  alloc::collections::btree::node::marker::LeafOrInternal>")
        //                         DW_AT_byte_size (0x10)
        //                         DW_AT_accessibility     (DW_ACCESS_protected)
        //                         DW_AT_alignment (8)

        // 0x000020f6:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x00002144 "alloc::collections::btree::node::marker::Owned")
        //                           DW_AT_name    ("BorrowType")

        // 0x000020ff:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x00001fb9 "alloc::string::String")
        //                           DW_AT_name    ("K")

        // 0x00002108:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x00005102 "i32")
        //                           DW_AT_name    ("V")

        // 0x00002111:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x0000214e "alloc::collections::btree::node::marker::LeafOrInternal")
        //                           DW_AT_name    ("Type")

        // 0x0000211a:             DW_TAG_member
        //                           DW_AT_name    ("height")
        //                           DW_AT_type    (0x0000009f "usize")
        //                           DW_AT_alignment       (8)
        //                           DW_AT_data_member_location    (0x08)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        // 0x00002126:             DW_TAG_member
        //                           DW_AT_name    ("node")
        //                           DW_AT_type    (0x00002361 "core::ptr::non_null::NonNull<alloc::collections::btree::node::LeafNode<a
        // lloc::string::String, i32>>")
        //                           DW_AT_alignment       (8)
        //                           DW_AT_data_member_location    (0x00)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        let (height_offset, (node_offset, node_entry)) = parse_children((
            member("height").then(offset()),
            member("node").then(offset().and(entry_type())),
        ))
        .parse(db, node_ref_type)?;

        // now we need to get the node entry layout

        // 0x00002159:           DW_TAG_structure_type
        //                         DW_AT_name      ("LeafNode<alloc::string::String, i32>")
        //                         DW_AT_byte_size (0x0140)
        //                         DW_AT_accessibility     (DW_ACCESS_private)
        //                         DW_AT_alignment (8)

        // 0x00002162:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x00001fb9 "alloc::string::String")
        //                           DW_AT_name    ("K")

        // 0x0000216b:             DW_TAG_template_type_parameter
        //                           DW_AT_type    (0x00005102 "i32")
        //                           DW_AT_name    ("V")

        // 0x00002174:             DW_TAG_member
        //                           DW_AT_name    ("parent")
        //                           DW_AT_type    (0x00003adc "core::option::Option<core::ptr::non_null::NonNull<alloc::collections::bt
        // ree::node::InternalNode<alloc::string::String, i32>>>")
        //                           DW_AT_alignment       (8)
        //                           DW_AT_data_member_location    (0x00)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        // 0x00002180:             DW_TAG_member
        //                           DW_AT_name    ("parent_idx")
        //                           DW_AT_type    (0x0000424f "core::mem::maybe_uninit::MaybeUninit<u16>")
        //                           DW_AT_alignment       (2)
        //                           DW_AT_data_member_location    (0x013c)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        // 0x0000218d:             DW_TAG_member
        //                           DW_AT_name    ("len")
        //                           DW_AT_type    (0x0000581a "u16")
        //                           DW_AT_alignment       (2)
        //                           DW_AT_data_member_location    (0x013e)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        // 0x0000219a:             DW_TAG_member
        //                           DW_AT_name    ("keys")
        //                           DW_AT_type    (0x00005821 "core::mem::maybe_uninit::MaybeUninit<alloc::string::String>[11]")
        //                           DW_AT_alignment       (8)
        //                           DW_AT_data_member_location    (0x08)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        // 0x000021a6:             DW_TAG_member
        //                           DW_AT_name    ("vals")
        //                           DW_AT_type    (0x0000582e "core::mem::maybe_uninit::MaybeUninit<i32>[11]")
        //                           DW_AT_alignment       (4)
        //                           DW_AT_data_member_location    (0x0110)
        //                           DW_AT_accessibility   (DW_ACCESS_private)

        Ok(MapDef {
            key_type,
            value_type,
            variant: MapVariant::BTreeMap {
                length_offset: 0, // Placeholder for length offset
                root_offset,
                height_offset,
                node_offset,
                node_entry: todo!(),
            },
        })
    }
}

pub fn root_node_type<'db>() -> impl Parser<'db, Arc<PartiallyParsedEnumVariant<'db>>> {
    is_member_tag(gimli::DW_TAG_structure_type).then(parse_option_entry())
}
