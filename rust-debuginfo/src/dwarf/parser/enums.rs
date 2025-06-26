//! Enum parser implementation using combinators

use super::Parser;
use super::{
    children::for_each_child,
    primitives::{
        attr, entry_type, is_member_tag, member_by_tag, optional_attr, resolve_type_shallow,
    },
};
use crate::database::Db;
use crate::dwarf::{Die, resolution::resolve_entry_type};
use rust_types::{Discriminant, DiscriminantType, EnumDef, EnumVariant};
use std::sync::Arc;

use anyhow::{Context as _, Result};

/// Parser for discriminant information
pub fn discriminant_parser<'db>() -> impl Parser<'db, Discriminant> {
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

/// Parser for enum types
pub fn enum_parser<'db>() -> impl Parser<'db, EnumDef> {
    struct EnumParser;

    impl<'db> Parser<'db, EnumDef> for EnumParser {
        fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<EnumDef> {
            let (name, size) = attr::<String>(gimli::DW_AT_name)
                .and(attr::<usize>(gimli::DW_AT_byte_size))
                .parse(db, entry)?;

            tracing::debug!("resolving enum type: {name} {}", entry.print(db));

            // Get the variant part
            let variants_entry = member_by_tag(gimli::DW_TAG_variant_part).parse(db, entry)?;

            // Parse discriminant info and variants
            let discriminant = discriminant_parser().parse(db, variants_entry)?;

            // Parse all variants
            let variant_dies =
                for_each_child(is_member_tag(gimli::DW_TAG_variant)).parse(db, variants_entry)?;
            let mut variants = Vec::new();

            for (i, variant_die) in variant_dies.into_iter().enumerate() {
                tracing::debug!("variant: {}", variant_die.print(db));

                // Get discriminant value or use index as fallback
                let discriminant = optional_attr::<usize>(gimli::DW_AT_discr_value)
                    .parse(db, variant_die)?
                    .unwrap_or(i) as u64;

                // Get the single member of the variant
                let member = member_by_tag(gimli::DW_TAG_member)
                    .context("variant part should have a single member")
                    .parse(db, variant_die)?;

                let (name, layout) = attr::<String>(gimli::DW_AT_name)
                    .and(entry_type().then(resolve_type_shallow()).map(Arc::new))
                    .parse(db, member)?;

                variants.push(EnumVariant {
                    name,
                    discriminant,
                    layout,
                });
            }

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
