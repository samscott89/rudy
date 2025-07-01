//! Enum parser implementation using combinators

use crate::DwarfDb;
use crate::parser::{
    Parser,
    children::{for_each_child, parse_children},
    combinators::all,
    primitives::{
        IsMember, attr, entry_type, is_member_tag, member, member_by_tag, offset, optional_attr,
        resolve_type_shallow,
    },
};
use crate::{Die, resolution::resolve_entry_type};
use rudy_types::{
    CEnumLayout, CEnumVariant, Discriminant, DiscriminantType, EnumLayout, EnumVariantLayout,
    PrimitiveLayout, TypeLayout,
};
use std::sync::Arc;

use anyhow::Result;

/// Parser for discriminant information
///
/// Should be called on a `DW_TAG_variant_part` DIE to extract the discriminant type and offset.
pub fn enum_discriminant<'db>() -> impl Parser<'db, Discriminant> {
    struct DiscriminantParser;

    impl<'db> Parser<'db, Discriminant> for DiscriminantParser {
        fn parse(&self, db: &'db dyn DwarfDb, variants_entry: Die<'db>) -> Result<Discriminant> {
            let (discr_die, offset) = optional_attr::<Die<'db>>(gimli::DW_AT_discr)
                .and(optional_attr::<usize>(gimli::DW_AT_data_member_location))
                .parse(db, variants_entry)?;

            if let Some(discr) = discr_die {
                // We have an explicit discriminant - resolve its type
                let discriminant_type = resolve_entry_type(db, discr)?;
                let ty = match discriminant_type {
                    rudy_types::TypeLayout::Primitive(rudy_types::PrimitiveLayout::Int(i)) => {
                        DiscriminantType::Int(i)
                    }
                    rudy_types::TypeLayout::Primitive(
                        rudy_types::PrimitiveLayout::UnsignedInt(u),
                    ) => DiscriminantType::UnsignedInt(u),
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
pub fn enum_variant<'db>() -> impl Parser<'db, EnumVariantLayout> {
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
            optional_attr::<i128>(gimli::DW_AT_discr_value).and(
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
            EnumVariantLayout {
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

/// Specialized version of `named_enum_variant` that parses a named enum variant
/// and then proceeds to parse it as a tuple variant, applying a parser to each field
///
/// Should be used with `parse_children` on a `DW_TAG_variant_part` DIE.
pub fn enum_named_tuple_variant<T>(variant_name: &str, parser: T) -> EnumNamedTupleVariant<T> {
    let variant_name = variant_name.to_string();

    EnumNamedTupleVariant {
        variant_name,
        parser,
    }
}

pub(super) struct PartiallyParsedEnumVariant<'db> {
    pub discriminant: Option<usize>,
    pub offset: usize,
    pub layout: Die<'db>,
}

/// Parses an enum variant by a specific name.
/// Should be used with `parse_children` on a `DW_TAG_variant_part` DIE.
///
///   DW_TAG_variant_part
///     DW_AT_discr       (0x00002440)
///     DW_TAG_member
///       DW_AT_type      (0x00004f86 "u32")
///       DW_AT_data_member_location      (0x00)
/// --> DW_TAG_variant
///       DW_AT_discr_value       (0x00)
///       DW_TAG_member
///         DW_AT_name    ("None")
///         DW_AT_type    (0x00002464 "core::option::Option<i32>::None<i32>")
///         DW_AT_data_member_location    (0x00)
/// --> DW_TAG_variant
///       DW_AT_discr_value       (0x01)
///       DW_TAG_member
///         DW_AT_name    ("Some")
///         DW_AT_type    (0x00002476 "core::option::Option<i32>::Some<i32>")
///         DW_AT_data_member_location    (0x00)
pub(super) fn named_enum_variant<'db>(
    variant_name: &str,
) -> impl Parser<'db, PartiallyParsedEnumVariant<'db>> {
    is_member_tag(gimli::DW_TAG_variant).then(
        optional_attr::<usize>(gimli::DW_AT_discr)
            .and(member(variant_name).then(all((offset(), entry_type()))))
            .map(
                |(discriminant, (offset, layout))| PartiallyParsedEnumVariant {
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
            fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> anyhow::Result<(Option<usize>, ($((usize, $T),)*))> {
                // --> DW_TAG_variant
                //       DW_AT_discr_value       (0x00)
                //       DW_TAG_member
                //         DW_AT_name    ("None")
                //         DW_AT_type    (0x00002464 "core::option::Option<i32>::None<i32>")
                //         DW_AT_data_member_location    (0x00)


                // we first need to resolve to the DW_TAG_variant -> DW_TAG_member with a matching name
                // and get the type entry for that member

                // e.g. in the case of "Some", we should get back
                // PartiallyParsedEnumVariant { name: "Some", discriminant: Some(1), offset: 0, layout: 0x00002476 }
                let variant = named_enum_variant(&self.variant_name).parse(db, entry)?;

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

/// Parser for enum types
///
/// Reference: https://github.com/rust-lang/rust/blob/3b97f1308ff72016a4aaa93fbe6d09d4d6427815/compiler/rustc_codegen_llvm/src/debuginfo/metadata/enums/native.rs
pub fn enum_def<'db>() -> impl Parser<'db, EnumLayout> {
    EnumParser
}

pub struct EnumParser;

impl<'db> Parser<'db, EnumLayout> for EnumParser {
    fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<EnumLayout> {
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

        Ok(EnumLayout {
            name,
            variants,
            size,
            discriminant,
        })
    }
}

/// Parser for C-style enum variants (DW_TAG_enumerator)
pub fn c_enum_variant<'db>() -> impl Parser<'db, CEnumVariant> {
    all((
        attr::<String>(gimli::DW_AT_name),
        attr::<i128>(gimli::DW_AT_const_value),
    ))
    .map(|(name, value)| CEnumVariant { name, value })
}

/// Parser for C-style enumeration types (DW_TAG_enumeration_type)
pub fn c_enum_def<'db>() -> impl Parser<'db, CEnumLayout> {
    struct CEnumParser;

    impl<'db> Parser<'db, CEnumLayout> for CEnumParser {
        fn parse(&self, db: &'db dyn DwarfDb, entry: Die<'db>) -> Result<CEnumLayout> {
            tracing::debug!("resolving C-style enum type: {}", entry.print(db));

            // Parse name, size, and underlying type
            let (name, size, underlying_type) = all((
                attr::<String>(gimli::DW_AT_name),
                attr::<usize>(gimli::DW_AT_byte_size),
                entry_type()
                    .then(resolve_type_shallow())
                    .map_res(|ty| match ty {
                        TypeLayout::Primitive(PrimitiveLayout::Int(i)) => {
                            Ok(DiscriminantType::Int(i))
                        }
                        TypeLayout::Primitive(PrimitiveLayout::UnsignedInt(u)) => {
                            Ok(DiscriminantType::UnsignedInt(u))
                        }
                        _ => Err(anyhow::anyhow!(
                            "C enum underlying type must be integer, got: {:?}",
                            ty
                        )),
                    }),
            ))
            .parse(db, entry)?;

            // Parse all enumerator variants
            let variants = for_each_child(c_enum_variant()).parse(db, entry)?;

            Ok(CEnumLayout {
                name,
                discriminant_type: underlying_type,
                variants,
                size,
            })
        }
    }

    CEnumParser
}
