//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::database::Db;
use crate::dwarf::Die;
use crate::typedef::{
    ArrayDef, DefKind, OptionDef, PointerDef, PrimitiveDef, StdDef, StrSliceDef, StructDef,
    StructField, TypeDef, UnsignedIntDef,
};
use crate::types::NameId;

/// Resolve the full type for a DIE entry
pub fn resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<TypeDef<'db>> {
    let Some(type_offset_val) = entry.get_attr(db, gimli::DW_AT_type) else {
        db.report_critical(format!("Failed to get type attribute"));
        return None;
    };

    let gimli::AttributeValue::UnitRef(type_offset) = type_offset_val else {
        db.report_critical(format!("Unexpected type offset: {type_offset_val:?}"));
        tracing::error!(
            "Unexpected type offset: {type_offset_val:?} for {}",
            entry.print(db)
        );
        return None;
    };

    let type_entry = entry.child_die(db, type_offset);
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<TypeDef<'db>> {
    let type_offset_val = entry.get_attr(db, gimli::DW_AT_type)?;

    let gimli::AttributeValue::UnitRef(type_offset) = type_offset_val else {
        db.report_critical(format!("Unexpected type offset: {type_offset_val:?}"));
        return None;
    };

    let type_entry = entry.child_die(db, type_offset);
    shallow_resolve_type(db, type_entry)
}

/// Resolve a primitive type by name
fn resolve_primitive_type<'db>(db: &'db dyn Db, name: NameId<'db>) -> TypeDef<'db> {
    match name.name(db).as_str() {
        "u64" => TypeDef::new(
            db,
            // Some(name),
            DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 8 })),
        ),
        _ => {
            db.report_critical(format!("unsupported type: {name:?}"));
            TypeDef::new(
                db,
                // Some(name),
                DefKind::Other {
                    name: name.as_path(db),
                },
            )
        }
    }
}

/// Resolve Option type structure
fn resolve_option_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<OptionDef<'db>> {
    // we have an option type -- but we still need to get the inner
    // type and we should double check the layout

    tracing::debug!("option: {}", entry.print(db));

    let mut some_type = None;

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
                            panic!("unexpected tag: {t}: {}", grandchild.print(db))
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
                                some_type = resolve_type_shallow(db, grandchild);
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
                                panic!("unexpected tag: {t}: {}", grandchild.print(db))
                            }
                        }
                    }
                }
            }
            t => {
                panic!("unexpected tag: {t}: {}", entry.print(db))
            }
        }
    }

    some_type.map(|t| OptionDef {
        inner_type: Arc::new(t),
    })
}

/// Resolves a type entry to a `Def` _if_ the target entry is one of the
/// support "builtin" types -- these are types that we manually resolve rather
/// than relying on the DWARF info to do so.
fn resolve_as_builtin_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<TypeDef<'db>> {
    match entry.tag(db) {
        gimli::DW_TAG_base_type => {
            // primitive type
            let name = entry.name(db)?;
            let name_id = NameId::new(db, vec![], name);
            let ty = resolve_primitive_type(db, name_id);
            Some(ty)
        }
        gimli::DW_TAG_pointer_type => {
            let Some(pointed_ty) = resolve_type_shallow(db, entry) else {
                db.report_critical(format!("Failed to get pointed type"));
                return None;
            };
            // let index = crate::index::build_index(db);
            // let canonical_name = index.data(db).die_to_type_name.get(&entry).copied();
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
                Some(n) if n == "&str" => {
                    let Some(size) = entry
                        .get_attr(db, gimli::DW_AT_byte_size)
                        .and_then(|attr| attr.udata_value())
                    else {
                        db.report_critical(format!("Failed to get size"));
                        return None;
                    };
                    let mut data_ptr_offset = None;
                    let mut len_offset = None;

                    for child in entry.children(db) {
                        if child.tag(db) == gimli::DW_TAG_member {
                            let Some(name) = child.name(db) else {
                                continue;
                            };
                            if name == "data_ptr" {
                                data_ptr_offset = Some(
                                    child
                                        .get_attr(db, gimli::DW_AT_data_member_location)?
                                        .udata_value()?
                                        as usize,
                                );
                            } else if name == "length" {
                                len_offset = Some(
                                    child
                                        .get_attr(db, gimli::DW_AT_data_member_location)?
                                        .udata_value()?
                                        as usize,
                                );
                            }
                        }
                    }

                    if let Some((data_ptr_offset, length_offset)) = data_ptr_offset.zip(len_offset)
                    {
                        // we have a string slice
                        // let canonical_name = crate::index::build_index(db)
                        //     .data(db)
                        //     .die_to_type_name
                        //     .get(&entry)
                        //     .copied();
                        return Some(TypeDef::new(
                            db,
                            // canonical_name,
                            DefKind::Primitive(PrimitiveDef::StrSlice(StrSliceDef {
                                data_ptr_offset,
                                length_offset,
                                size: size as usize,
                            })),
                        ));
                    } else {
                        db.report_critical(format!("Failed to find data/len offsets"));
                        return None;
                    }
                }
                Some(n) if n.starts_with("Option<") => {
                    // this is _probably_ the option type, but let's double check:
                    // let index = crate::index::build_index(db);
                    // let Some(canonical_name) = index.data(db).die_to_type_name.get(&entry).copied()
                    // else {
                    //     tracing::warn!("failed to get canonical name for option type");
                    //     return None;
                    // };
                    // if !matches!(&canonical_name.path(db)[..], [p0, p1] if p0 == "core" && p1 == "option")
                    // {
                    //     tracing::debug!("this is _not_ the option type from core");
                    //     return None;
                    // }

                    let def = resolve_option_type(db, entry);
                    let Some(def) = def else {
                        db.report_critical(format!("Failed to resolve option type"));
                        return None;
                    };
                    return Some(TypeDef::new(
                        db,
                        // Some(canonical_name),
                        DefKind::Std(StdDef::Option(def)),
                    ));
                }
                _ => None,
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
            let Some(pointed_ty) = resolve_type_shallow(db, entry) else {
                db.report_critical(format!("Failed to get pointed type"));
                return None;
            };
            // let index = crate::index::build_index(db);
            // let canonical_name = index.data(db).die_to_type_name.get(&entry).copied();

            // get the child with the size
            let children = entry.children(db);
            let Some(size_child) = children
                .iter()
                .find(|child| child.tag(db) == gimli::DW_TAG_subrange_type)
            else {
                tracing::warn!("no size child found for array");
                return None;
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
        _ => None,
    }
}

/// "Shallow" resolve a type -- if it's a primitive value, then
/// we'll return that directly. Otherwise, return an alias to some other
/// type entry (if we can find it).
pub fn shallow_resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<TypeDef<'db>> {
    let ty = if let Some(builtin_ty) = resolve_as_builtin_type(db, entry) {
        // we have a builtin type -- use it
        tracing::debug!("builtin: {builtin_ty:?}");
        builtin_ty
    } else {
        TypeDef::new(db, DefKind::Alias(entry))
    };
    Some(ty)
}

/// Fully resolve a type from a DWARF DIE entry
#[salsa::tracked]
pub fn resolve_type_offset<'db>(db: &'db dyn Db, entry: Die<'db>) -> Option<TypeDef<'db>> {
    if let Some(def) = resolve_as_builtin_type(db, entry) {
        return Some(def);
    }

    match entry.tag(db) {
        gimli::DW_TAG_base_type => {
            // unhandled primitive type
            db.report_critical(format!("Primitive type not handled: {}", entry.print(db)));
            return None;
        }
        gimli::DW_TAG_structure_type => {
            // DWARF info says struct, but in practice this could also be an enum

            let is_enum = entry
                .children(db)
                .iter()
                .any(|c| c.tag(db) == gimli::DW_TAG_variant_part);

            if is_enum {
                todo!("handle enums")
            } else {
                // we have a struct type and it's _not_ a builtin -- we'll handle it now

                // let index = crate::index::build_index(db);
                // let type_name = index.data(db).die_to_type_name.get(&entry).copied();
                // get some basic info
                let name = entry.name(db)?;
                let size = entry.get_attr(db, gimli::DW_AT_byte_size)?.udata_value()?;
                let alignment = entry.get_attr(db, gimli::DW_AT_alignment)?.udata_value()?;

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

                    let type_offset_val = child.get_attr(db, gimli::DW_AT_type)?;
                    let gimli::AttributeValue::UnitRef(type_offset) = type_offset_val else {
                        db.report_critical(format!("Unexpected type offset: {type_offset_val:?}"));
                        return None;
                    };

                    let type_entry = child.child_die(db, type_offset);

                    if let Some(ty) = shallow_resolve_type(db, type_entry) {
                        fields.push(StructField {
                            name: field_name,
                            offset: offset as usize,
                            ty: Arc::new(ty),
                        });
                    }
                }
                return Some(TypeDef::new(
                    db,
                    // type_name,
                    DefKind::Struct(StructDef {
                        name,
                        fields,
                        size: size as usize,
                        alignment: alignment as usize,
                    }),
                ));
            }
        }
        t => {
            db.report_critical(format!("unsupported type: {}", entry.print(db)));
            tracing::debug!("unsupported type entry: {t}: {entry:#?}");
        }
    }

    None
}
