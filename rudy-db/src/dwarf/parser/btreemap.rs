//! BTreeMap parser implementation using combinators
//!
//! Inspired by the `BTreeMapProvider` in [gdb_provider.py](https://github.com/rust-lang/rust/blob/513999b936c37902120380f4171963d1f1d80347/src/etc/gdb_providers.py#L4)
//!
//!   Overview: BTreeMap stores key-value pairs in a B-tree structure with internal nodes and leaf nodes. The tree has a
//! height, and we traverse from the root down to the leaves.
//!
//! Algorithm:
//! 1. Start with the BTreeMap's root node and height
//! 2. If the map is empty (length = 0), return immediately
//! 3. Extract the root node pointer from the Option wrapper
//! 4. Recursively traverse the tree starting from the root with the given height
//! 5. For each node:
//! - If it's an internal node (height > 0):
//!     - Cast to InternalNode type to access edges
//!   - Recursively traverse each child edge before processing keys/values
//! - Process the node's key-value pairs:
//!     - Read the len field to know how many pairs are in this node
//!   - Iterate through indices 0 to len-1
//!   - For each index, yield the key and value from the arrays
//!
//!
//! Key details:
//! - Internal nodes have edges (child pointers) in addition to keys/values
//! - Leaf nodes only have keys/values
//! - The tree is traversed in-order (left edge, key/value, right edge)
//! - Zero-sized types need special handling

use super::Parser;
use super::primitives::entry_type;
use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::parser::children::parse_children;
use crate::dwarf::parser::option::parse_option_entry;
use crate::dwarf::parser::pointers::nonnull;
use crate::dwarf::parser::primitives::{is_member, member, offset, resolved_generic};
use rust_types::{BTreeNodeLayout, BTreeRootLayout, MapLayout, MapVariant};

use anyhow::Result;

/// Parser for btree BTreeMap layout
pub fn btree_map<'db>() -> BTreeMapParser {
    BTreeMapParser
}

pub struct BTreeMapParser;

impl<'db> Parser<'db, MapLayout> for BTreeMapParser {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<MapLayout> {
        tracing::debug!("resolving btree map type: {}", entry.print(db));

        // Parse key type, value type, root field, and length field from BTreeMap
        let (key_type, value_type, (root_offset, root_option_type), length_offset) =
            parse_children((
                resolved_generic("K"),
                resolved_generic("V"),
                is_member("root").then(offset().and(entry_type())),
                is_member("length").then(offset()),
            ))
            .parse(db, entry)?;

        tracing::debug!("resolving root field: {}", root_option_type.print(db));

        // Parse the Option<NodeRef> to get the Some variant which contains the NodeRef
        let (_, _, _, some_variant) = parse_option_entry().parse(db, root_option_type)?;
        let node_ref_type = some_variant.layout;

        tracing::debug!("resolving node ref field: {}", node_ref_type.print(db));

        // Parse the NodeRef struct to get height and node pointer offsets
        let (height_offset, (node_offset, node_ptr_type)) = parse_children((
            is_member("height").then(offset()),
            is_member("node").then(offset().and(entry_type())),
        ))
        .parse(db, node_ref_type)?;

        // Create the root layout
        let root_layout = BTreeRootLayout {
            node_offset,
            height_offset,
        };

        // The node pointer is a NonNull<LeafNode<K, V>>, resolve it to get the leaf node type
        let leaf_node_type = nonnull().parse(db, node_ptr_type)?;

        tracing::debug!("resolving leaf node type: {}", leaf_node_type.print(db));

        // Parse the LeafNode structure to get offsets
        let (len_offset, keys_offset, vals_offset) = parse_children((
            is_member("len").then(offset()),
            is_member("keys").then(offset()),
            is_member("vals").then(offset()),
        ))
        .parse(db, leaf_node_type)?;

        // To get the InternalNode type, we need to find the parent field which contains
        // Option<NonNull<InternalNode<K, V>>>. We'll extract this type.
        let parent_option_type = member("parent")
            .then(entry_type())
            .parse(db, leaf_node_type)?;

        // Parse the Option to get the Some variant
        let (_, _, _, parent_some_variant) = parse_option_entry().parse(db, parent_option_type)?;

        // The Some variant contains NonNull<InternalNode<K, V>>, resolve it
        let internal_node_type = nonnull().parse(db, parent_some_variant.layout)?;

        // For InternalNode, we need the edges offset. The edges field is specific to InternalNode.
        let edges_offset = member("edges")
            .then(offset())
            .parse(db, internal_node_type)?;

        // Create the node layout
        let node_layout = BTreeNodeLayout {
            keys_offset,
            vals_offset,
            len_offset,
            edges_offset,
        };

        Ok(MapLayout {
            key_type,
            value_type,
            variant: MapVariant::BTreeMap {
                length_offset,
                root_offset,
                root_layout,
                node_layout,
            },
        })
    }
}
