//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::database::Db;
use crate::dwarf::Die;
use crate::typedef::{
    ArrayDef, DefKind, FloatDef, IntDef, OptionDef, PointerDef, PrimitiveDef, StdDef, StrSliceDef,
    StructDef, StructField, TypeDef, UnitDef, UnsignedIntDef,
};
use crate::types::NameId;

use anyhow::Context;

type Result<T> = std::result::Result<T, super::Error>;

/// Resolve the full type for a DIE entry
pub fn resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef<'db>> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef<'db>> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve a primitive type by name
fn resolve_primitive_type<'db>(db: &'db dyn Db, name: NameId<'db>) -> TypeDef<'db> {
    use PrimitiveDef::*;
    let primitive_def = match name.name(db).as_str() {
        "u8" => UnsignedInt(UnsignedIntDef { size: 1 }),
        "u16" => UnsignedInt(UnsignedIntDef { size: 2 }),
        "u32" => UnsignedInt(UnsignedIntDef { size: 4 }),
        "u64" => UnsignedInt(UnsignedIntDef { size: 8 }),
        "i8" => Int(IntDef { size: 1 }),
        "i16" => Int(IntDef { size: 2 }),
        "i32" => Int(IntDef { size: 4 }),
        "i64" => Int(IntDef { size: 8 }),
        "usize" => UnsignedInt(UnsignedIntDef {
            // TODO(Sam): this should be `target_pointer_width` from the target triple
            size: std::mem::size_of::<usize>(),
        }),
        "isize" => Int(IntDef {
            size: std::mem::size_of::<isize>(),
        }),
        "f32" => Float(FloatDef { size: 4 }),
        "f64" => Float(FloatDef { size: 8 }),
        "bool" => Bool(()),
        "char" => Char(()),
        "()" => Unit(UnitDef),

        _ => {
            db.report_critical(format!("unsupported type: {name:?}"));
            return TypeDef::new(
                db,
                DefKind::Other {
                    name: name.as_path(db),
                },
            );
        }
    };
    TypeDef::new(db, DefKind::Primitive(primitive_def))
}

/// Resolve Option type structure
fn resolve_option_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<OptionDef<'db>> {
    // we have an option type -- but we still need to get the inner
    // type and we should double check the layout

    tracing::debug!("option: {}", entry.print(db));

    for child in entry.children(db) {
        tracing::debug!("option child: {}", child.print(db));

        match child.tag(db) {
            gimli::DW_TAG_variant_part => {
                // this tells us how the variants are laid out
                for grandchild in child.children(db) {
                    match grandchild.tag(db) {
                        gimli::DW_TAG_member => {
                            // the disciminant
                            debug_assert_eq!(
                                grandchild
                                    .get_attr(db, gimli::DW_AT_data_member_location)
                                    .and_then(|attr| attr.udata_value())
                                    .unwrap_or(u64::MAX),
                                0
                            )
                        }
                        gimli::DW_TAG_variant => {
                            // one of the variants
                            if let Some(variant_entry) = grandchild.children(db).first() {
                                // check the offset is at 0
                                debug_assert_eq!(
                                    variant_entry
                                        .get_attr(db, gimli::DW_AT_data_member_location)
                                        .and_then(|attr| attr.udata_value())
                                        .unwrap_or(u64::MAX),
                                    0
                                )
                            } else {
                                tracing::warn!("no member found for variant");
                            }
                        }
                        t => {
                            return Err(grandchild
                                .format_with_location(db, format!("unexpected tag: {t}"))
                                .into());
                        }
                    }
                }
            }
            gimli::DW_TAG_structure_type => {
                // the type definitions
                let Some(name) = child.name(db) else {
                    continue;
                };
                if name == "Some" {
                    for grandchild in child.children(db) {
                        match grandchild.tag(db) {
                            gimli::DW_TAG_template_type_parameter => {
                                // formally, this tells us the type
                                // of the option generic `T`
                                return Ok(OptionDef {
                                    inner_type: Arc::new(resolve_type_shallow(db, grandchild)?),
                                });
                            }
                            gimli::DW_TAG_member => {
                                // formally, this is a reference to the tuple field(s)
                                // of the enum -- in the case of option we should have
                                // a single one that points at the field.
                                debug_assert_eq!(
                                    grandchild
                                        .get_attr(db, gimli::DW_AT_data_member_location)
                                        .and_then(|attr| attr.udata_value())
                                        .unwrap_or(u64::MAX),
                                    0
                                )
                            }
                            t => {
                                return Err(grandchild
                                    .format_with_location(db, format!("unexpected tag: {t}"))
                                    .into());
                            }
                        }
                    }
                }
            }
            t => {
                return Err(entry
                    .format_with_location(db, format!("unexpected tag: {t}"))
                    .into());
            }
        }
    }

    // if we got here, then we should have found the inner type
    Err(entry
        .format_with_location(db, "failed to find option type")
        .into())
}

fn resolve_str_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef<'db>> {
    let Some(size) = entry
        .get_attr(db, gimli::DW_AT_byte_size)
        .and_then(|attr| attr.udata_value())
    else {
        return Err(entry
            .format_with_location(db, "struct size not found")
            .into());
    };
    let mut data_ptr_offset = None;
    let mut length_offset = None;

    for child in entry.children(db) {
        if child.tag(db) == gimli::DW_TAG_member {
            let Some(name) = child.name(db) else {
                continue;
            };
            if name == "data_ptr" {
                if let Some(offset) = child
                    .get_attr(db, gimli::DW_AT_data_member_location)
                    .and_then(|attr| attr.udata_value().map(|v| v as usize))
                {
                    data_ptr_offset = Some(offset);
                } else {
                    return Err(child
                        .format_with_location(db, "could not read data_ptr offset")
                        .into());
                }
            } else if name == "length" {
                if let Some(offset) = child
                    .get_attr(db, gimli::DW_AT_data_member_location)
                    .and_then(|attr| attr.udata_value().map(|v| v as usize))
                {
                    length_offset = Some(offset);
                } else {
                    return Err(child
                        .format_with_location(db, "could not read length offset")
                        .into());
                }
            }
        }
    }

    let Some(data_ptr_offset) = data_ptr_offset else {
        return Err(entry
            .format_with_location(db, "data_ptr offset not found")
            .into());
    };
    let Some(length_offset) = length_offset else {
        return Err(entry
            .format_with_location(db, "length offset not found")
            .into());
    };
    Ok(TypeDef::new(
        db,
        // canonical_name,
        DefKind::Primitive(PrimitiveDef::StrSlice(StrSliceDef {
            data_ptr_offset,
            length_offset,
            size: size as usize,
        })),
    ))
}

/// Resolves a type entry to a `Def` _if_ the target entry is one of the
/// support "builtin" types -- these are types that we manually resolve rather
/// than relying on the DWARF info to do so.
fn resolve_as_builtin_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<Option<TypeDef<'db>>> {
    Ok(match entry.tag(db) {
        gimli::DW_TAG_base_type => {
            // primitive type
            let Some(name) = entry.name(db) else {
                return Err(entry.format_with_location(db, "type name not found").into());
            };
            let name_id = NameId::new(db, vec![], name);
            let ty = resolve_primitive_type(db, name_id);
            Some(ty)
        }
        gimli::DW_TAG_pointer_type => {
            let pointed_ty = resolve_type_shallow(db, entry)?;
            Some(TypeDef::new(
                db,
                // canonical_name,
                DefKind::Primitive(PrimitiveDef::Pointer(PointerDef {
                    pointed_type: Arc::new(pointed_ty),
                })),
            ))
        }
        gimli::DW_TAG_structure_type => {
            match entry.name(db) {
                Some(n) if n == "&str" => Some(resolve_str_type(db, entry)?),
                Some(n) if n.starts_with("Option<") => {
                    // this is _probably_ the option type, but would be good to double check
                    let def = resolve_option_type(db, entry)?;
                    Some(TypeDef::new(
                        db,
                        // Some(canonical_name),
                        DefKind::Std(StdDef::Option(def)),
                    ))
                }
                Some(n) => {
                    tracing::trace!(
                        "not handled as a builtin: {n} {}",
                        entry.format_with_location(db, "")
                    );
                    None
                }
                _ => {
                    tracing::trace!(
                        "{}",
                        entry.format_with_location(db, "no name found for struct")
                    );
                    None
                }
            }
        }
        gimli::DW_TAG_array_type => {
            // Typical array type entry:
            // 0x000001c4:   DW_TAG_array_type
            //                 DW_AT_type      (0x00000063 "simple_test::TestStruct")

            // 0x000001c9:     DW_TAG_subrange_type
            //                  DW_AT_type    (0x000001d1 "__ARRAY_SIZE_TYPE__")
            //                  DW_AT_lower_bound     (0)
            //                  DW_AT_count   (0x01)

            // 0x000001d0:     NULL

            // 0x000001d1:   DW_TAG_base_type
            //                  DW_AT_name      ("__ARRAY_SIZE_TYPE__")
            //                  DW_AT_byte_size (0x08)
            //                  DW_AT_encoding  (DW_ATE_unsigned)
            let pointed_ty = resolve_type_shallow(db, entry)?;

            // get the child with the size
            let children = entry.children(db);
            let Some(size_child) = children
                .iter()
                .find(|child| child.tag(db) == gimli::DW_TAG_subrange_type)
            else {
                return Err(entry
                    .format_with_location(db, "array type does not have a size child")
                    .into());
            };

            let count = size_child
                .get_attr(db, gimli::DW_AT_count)
                .and_then(|attr| attr.udata_value())
                .unwrap_or(0);

            let lower_bound = size_child
                .get_attr(db, gimli::DW_AT_lower_bound)
                .and_then(|attr| attr.udata_value())
                .unwrap_or(0);

            debug_assert_eq!(lower_bound, 0);

            Some(TypeDef::new(
                db,
                // canonical_name,
                DefKind::Primitive(PrimitiveDef::Array(ArrayDef {
                    element_type: Arc::new(pointed_ty),
                    length: count as usize,
                })),
            ))
        }
        t => {
            tracing::trace!(
                "not handled as a builtin: {t} {}",
                entry.format_with_location(db, "")
            );
            None
        }
    })
}

/// "Shallow" resolve a type -- if it's a primitive value, then
/// we'll return that directly. Otherwise, return an alias to some other
/// type entry (if we can find it).
pub fn shallow_resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef<'db>> {
    Ok(
        if let Some(builtin_ty) = resolve_as_builtin_type(db, entry)? {
            // we have a builtin type -- use it
            tracing::debug!("builtin: {builtin_ty:?}");
            builtin_ty
        } else {
            TypeDef::new(db, DefKind::Alias(entry))
        },
    )
}

/// Fully resolve a type from a DWARF DIE entry
#[salsa::tracked]
pub fn resolve_type_offset<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef<'db>> {
    if let Some(def) = resolve_as_builtin_type(db, entry)? {
        return Ok(def);
    }

    Ok(match entry.tag(db) {
        gimli::DW_TAG_base_type => {
            // unhandled primitive type
            tracing::debug!("unhandled primitive type: {}", entry.print(db));
            return Err(entry
                .format_with_location(db, "Primitive type not handled")
                .into());
        }
        gimli::DW_TAG_structure_type => {
            // DWARF info says struct, but in practice this could also be an enum

            let is_enum = entry
                .children(db)
                .iter()
                .any(|c| c.tag(db) == gimli::DW_TAG_variant_part);

            if is_enum {
                return Err(entry
                    .format_with_location(db, "enums are not yet supported")
                    .into());
            } else {
                // we have a struct type and it's _not_ a builtin -- we'll handle it now

                // let index = crate::index::build_index(db);
                // let type_name = index.data(db).die_to_type_name.get(&entry).copied();
                // get some basic info
                let Some(name) = entry.name(db) else {
                    return Err(entry
                        .format_with_location(db, "name not found for struct")
                        .into());
                };
                let Some(size) = entry
                    .get_attr(db, gimli::DW_AT_byte_size)
                    .and_then(|v| v.udata_value())
                else {
                    return Err(entry
                        .format_with_location(db, "struct size not found")
                        .into());
                };
                let Some(alignment) = entry
                    .get_attr(db, gimli::DW_AT_alignment)
                    .and_then(|v| v.udata_value())
                else {
                    return Err(entry
                        .format_with_location(db, "struct alignment not found")
                        .into());
                };

                // iterate children to get fields
                let mut fields = vec![];
                for child in entry.children(db) {
                    if child.tag(db) != gimli::DW_TAG_member {
                        tracing::debug!("skipping non-member entry: {entry:#?}");
                        continue;
                    }
                    let Some(field_name) = child.name(db) else {
                        db.report_critical(format!("Failed to parse field name"));
                        continue;
                    };

                    let Some(offset) = child
                        .get_attr(db, gimli::DW_AT_data_member_location)
                        .and_then(|attr| attr.udata_value())
                    else {
                        db.report_critical(format!("Failed to parse field offset"));
                        continue;
                    };

                    let Some(type_entry) = child.get_unit_ref(db, gimli::DW_AT_type)? else {
                        return Err(child
                            .format_with_location(
                                db,
                                format!("failed to get type for `{field_name}`"),
                            )
                            .into());
                    };

                    let ty = shallow_resolve_type(db, type_entry)?;

                    fields.push(StructField {
                        name: field_name,
                        offset: offset as usize,
                        ty: Arc::new(ty),
                    });
                }
                TypeDef::new(
                    db,
                    // type_name,
                    DefKind::Struct(StructDef {
                        name,
                        fields,
                        size: size as usize,
                        alignment: alignment as usize,
                    }),
                )
            }
        }
        t => {
            return Err(entry
                .format_with_location(db, format!("unsupported type: {t}"))
                .into());
        }
    })
}

// impl From<std::io::Error> for Error {
//     fn from(err: std::io::Error) -> Self {
//         Error::Io(Arc::new(err))
//     }
// }

// impl From<object::read::Error> for Error {
//     fn from(err: object::read::Error) -> Self {
//         Error::ObjectParseError(err)
//     }
// }
