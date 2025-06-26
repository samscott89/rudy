//! Enum parser implementation using combinators

use super::Parser;
use super::{
    children::for_each_child,
    primitives::{
        attr, entry_type, is_member_tag, member_by_tag, optional_attr, resolve_type_shallow,
    },
};
use crate::database::Db;
use crate::dwarf::parser::{combinators::all, primitives::offset};
use crate::dwarf::{Die, resolution::resolve_entry_type};
use rust_types::{Discriminant, DiscriminantType, EnumDef, EnumVariant};
use std::sync::Arc;

use anyhow::Result;

/// Parser for discriminant information
pub fn enum_discriminant<'db>() -> impl Parser<'db, Discriminant> {
    struct DiscriminantParser;

    impl<'db> Parser<'db, Discriminant> for DiscriminantParser {
        fn parse(&self, db: &'db dyn Db, variants_entry: Die<'db>) -> Result<Discriminant> {
            let (discr_die, offset) = optional_attr::<Die<'db>>(gimli::DW_AT_discr)
                .and(optional_attr::<usize>(gimli::DW_AT_data_member_location))
                .parse(db, variants_entry)?;

            if let Some(discr) = discr_die {
                // We have an explicit discriminant - resolve its type
                let discriminant_type = resolve_entry_type(db, discr)?;
                let ty = match discriminant_type {
                    rust_types::TypeDef::Primitive(rust_types::PrimitiveDef::Int(i)) => {
                        DiscriminantType::Int(i)
                    }
                    rust_types::TypeDef::Primitive(rust_types::PrimitiveDef::UnsignedInt(u)) => {
                        DiscriminantType::UnsignedInt(u)
                    }
                    _ => {
                        tracing::warn!(
                            "discriminant type is not an integer: {discriminant_type:?} {}",
                            discr.location(db)
                        );
                        DiscriminantType::Implicit
                    }
                };
                Ok(Discriminant {
                    ty,
                    offset: offset.unwrap_or(0),
                })
            } else {
                // No explicit discriminant, so we assume it's implicit
                Ok(Discriminant {
                    ty: DiscriminantType::Implicit,
                    offset: 0,
                })
            }
        }
    }

    DiscriminantParser
}

/// Parser for enum variants
pub fn enum_variant<'db>() -> impl Parser<'db, EnumVariant> {
    // 0x000002f5:           DW_TAG_variant
    //                         DW_AT_discr_value       (0x00)

    // 0x000002f7:             DW_TAG_member
    //                         DW_AT_name    ("None")
    //                         DW_AT_type    (0x00000312 "core::option::Option<u32>::None<u32>")
    //                         DW_AT_alignment       (4)
    //                         DW_AT_data_member_location    (0x00)

    // check we're in a variant DIE
    is_member_tag(gimli::DW_TAG_variant)
        .then(
            // we _may_ have a discriminant value
            optional_attr::<usize>(gimli::DW_AT_discr_value).and(
                // and we must have a member
                member_by_tag(gimli::DW_TAG_member).then(
                    // which has a name, offset, and type
                    all((
                        attr::<String>(gimli::DW_AT_name),
                        offset(),
                        entry_type().then(resolve_type_shallow()).map(Arc::new),
                    )),
                ),
            ),
        )
        .map(|(discriminant, (name, offset, layout))| {
            // Generally it seems like the variants should have offset 0, and we get the
            // "real" offsets from variant layouts themselves. But we need to verify this
            debug_assert_eq!(offset, 0, "enum variants should not have offsets");
            EnumVariant {
                name,
                discriminant,
                layout,
            }
        })
}

/// Parser for enum types
pub fn enum_def<'db>() -> impl Parser<'db, EnumDef> {
    struct EnumParser;

    impl<'db> Parser<'db, EnumDef> for EnumParser {
        fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<EnumDef> {
            tracing::debug!("resolving enum type: {}", entry.print(db));

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
            let variants = for_each_child(enum_variant()).parse(db, variants_entry)?;

            Ok(EnumDef {
                name,
                variants,
                size,
                discriminant,
            })
        }
    }

    EnumParser
}
