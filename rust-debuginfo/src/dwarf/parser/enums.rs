//! Enum parser implementation using combinators

use super::Parser;
use super::{
    children::for_each_child,
    primitives::{
        attr, entry_type, is_member_tag, member_by_tag, optional_attr, resolve_type_shallow,
    },
};
use crate::database::Db;
use crate::dwarf::parser::{
    children::parse_children,
    combinators::all,
    primitives::{IsMember, member, offset},
};
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

pub struct EnumNamedTupleVariant<T> {
    variant_name: String,
    parser: T,
}

/// Specialized enum variant parser for finding a named variant with a single tuple field
/// Returns (variant_offset, (tuple of fields)) for the single field in the variant
pub fn enum_named_tuple_variant<'db, T>(variant_name: &str, parser: T) -> EnumNamedTupleVariant<T> {
    let variant_name = variant_name.to_string();

    EnumNamedTupleVariant {
        variant_name,
        parser,
    }
}

struct PartiallyParsedEnumVariant<'db> {
    #[allow(dead_code)]
    pub name: String,
    pub discriminant: Option<usize>,
    pub offset: usize,
    pub layout: Die<'db>,
}

fn named_enum_variant<'db>(
    variant_name: &str,
) -> impl Parser<'db, PartiallyParsedEnumVariant<'db>> {
    is_member_tag(gimli::DW_TAG_variant).then(
        optional_attr::<usize>(gimli::DW_AT_discr)
            .and(member(variant_name).then(all((
                attr::<String>(gimli::DW_AT_name),
                offset(),
                entry_type(),
            ))))
            .map(
                |(discriminant, (name, offset, layout))| PartiallyParsedEnumVariant {
                    name,
                    discriminant,
                    offset,
                    layout,
                },
            ),
    )
}

macro_rules! impl_parse_enum_named_tuple_variant_for_tuples {
    (
        $($P:ident, $T:ident, $idx:tt),*
    ) => {
        impl<'db, $($T, $P,)*> Parser<'db, (Option<usize>, ($((usize, $T),)*))> for EnumNamedTupleVariant<($($P,)*)>
        where
            $($P: Parser<'db, $T>),*
        {
            fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> anyhow::Result<(Option<usize>, ($((usize, $T),)*))> {
                // We start with a structure like
                // 0x0000243b:         DW_TAG_variant_part
                //                       DW_AT_discr       (0x00002440)

                // 0x00002440:           DW_TAG_member
                //                         DW_AT_type      (0x00004f86 "u32")
                //                         DW_AT_data_member_location      (0x00)

                // 0x00002447:           DW_TAG_variant
                //                         DW_AT_discr_value       (0x00)

                // 0x00002449:             DW_TAG_member
                //                           DW_AT_name    ("None")
                //                           DW_AT_type    (0x00002464 "core::option::Option<i32>::None<i32>")
                //                           DW_AT_data_member_location    (0x00)

                // 0x00002455:           DW_TAG_variant
                //                         DW_AT_discr_value       (0x01)

                // 0x00002457:             DW_TAG_member
                //                           DW_AT_name    ("Some")
                //                           DW_AT_type    (0x00002476 "core::option::Option<i32>::Some<i32>")
                //                           DW_AT_data_member_location    (0x00)


                // we first need to resolve to the DW_TAG_variant -> DW_TAG_member with a matching name
                // and get the type entry for that member

                // e.g. in the case of "Some", we should get back
                // PartiallyParsedEnumVariant { name: "Some", discriminant: Some(1), offset: 0, layout: 0x00002476 }
                let (variant, ) = parse_children((
                    named_enum_variant(&self.variant_name),
                ))
                    .parse(db, entry)?;

                debug_assert_eq!(variant.offset, 0, "enum variants should not have offsets");

                // next, e.g. in the case of "Some", we get a structure type for the enum variant
                // 0x00002476:         DW_TAG_structure_type
                //                       DW_AT_name        ("Some")
                //                       DW_AT_byte_size   (0x08)
                //                       DW_AT_accessibility       (DW_ACCESS_public)
                //                       DW_AT_alignment   (4)

                // 0x0000247e:           DW_TAG_template_type_parameter
                //                         DW_AT_type      (0x000036f7 "i32")
                //                         DW_AT_name      ("T")

                // 0x00002487:           DW_TAG_member
                //                         DW_AT_name      ("__0")
                //                         DW_AT_type      (0x000036f7 "i32")
                //                         DW_AT_alignment (4)
                //                         DW_AT_data_member_location      (0x04)
                //                         DW_AT_accessibility     (DW_ACCESS_public)

                // parses each subfield as a member called `__0`, `__1`, etc.
                let field_parser = (
                    $(
                        IsMember { expected_name: format!("__{}", $idx) }
                            .then(all((
                                offset(),
                                entry_type().then(&self.parser.$idx)
                            ))),
                    )*
                );

                parse_children(field_parser).map(|fields| {
                    // If we have a discriminant, return it, otherwise None
                    let discriminant = variant.discriminant;
                    (discriminant, fields)
                }).parse(db, variant.layout)
            }
        }
    };
}

impl_parse_enum_named_tuple_variant_for_tuples!();
impl_parse_enum_named_tuple_variant_for_tuples!(P0, T0, 0);
impl_parse_enum_named_tuple_variant_for_tuples!(P0, T0, 0, P1, T1, 1);
impl_parse_enum_named_tuple_variant_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2);
impl_parse_enum_named_tuple_variant_for_tuples!(P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3);
impl_parse_enum_named_tuple_variant_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4
);
impl_parse_enum_named_tuple_variant_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5
);
impl_parse_enum_named_tuple_variant_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6
);
impl_parse_enum_named_tuple_variant_for_tuples!(
    P0, T0, 0, P1, T1, 1, P2, T2, 2, P3, T3, 3, P4, T4, 4, P5, T5, 5, P6, T6, 6, P7, T7, 7
);

// impl<'db, T> Parser<'db, (usize, Arc<TypeDef>)> for EnumNamedTupleVariant<T> {
//     fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<(usize, Arc<TypeDef>)> {
//         // check we're in a variant DIE
//         is_member_tag(gimli::DW_TAG_variant)
//             // then get the member with the specified name
//             .then(member(&self.variant_name))
//             .then(offset().and(entry_type().map(|_| todo!())))
//             .parse(db, entry)
//     }
// }

/// Parser for enum types
pub fn enum_def<'db>() -> impl Parser<'db, EnumDef> {
    EnumParser
}

pub struct EnumParser;

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
