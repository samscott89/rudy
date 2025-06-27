//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::index::get_die_typename;
use crate::dwarf::parser::btreemap::btree_map;
use crate::dwarf::parser::option::option_def;
use crate::dwarf::parser::result::result_def;
use crate::dwarf::parser::{
    Parser,
    children::parse_children,
    enums::{c_enum_def, enum_def},
    hashmap::hashbrown_map,
    primitives::{entry_type, member, resolved_generic},
    vec::vec,
};
use crate::file::DebugFile;
use rudy_types::*;

use anyhow::Context;
use gimli::{DebugInfoOffset, UnitOffset};

type Result<T> = std::result::Result<T, super::Error>;

/// Resolve the full type for a DIE entry
pub fn resolve_entry_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeLayout> {
    let type_entry = entry.get_referenced_entry(db, gimli::DW_AT_type)?;
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_entry_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeLayout> {
    let type_entry = entry.get_referenced_entry(db, gimli::DW_AT_type)?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve String type layout from DWARF
fn resolve_string_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<StringLayout> {
    // Get the vec field type and parse it as a Vec
    Ok(member("vec")
        .then(entry_type())
        .then(vec())
        .parse(db, entry)
        .map(StringLayout)?)
}

/// Resolve Map type layout from DWARF
fn resolve_map_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    variant: MapVariant,
) -> Result<MapLayout> {
    let (key_type, value_type) =
        parse_children((resolved_generic("K"), resolved_generic("V"))).parse(db, entry)?;

    match variant {
        MapVariant::HashMap { .. } => {
            // we'll detect a std hashmap by looking for the "base" member
            let hashbrown_entry =
                if let Ok(base) = member("base").then(entry_type()).parse(db, entry) {
                    base
                } else {
                    entry
                };

            // Now we can parse the hashbrown hashmap layout
            let variant = hashbrown_map()
                .parse(db, hashbrown_entry)
                .context("failed to parse hashbrown hashmap layout")?;

            Ok(MapLayout {
                key_type,
                value_type,
                variant,
            })
        }
        MapVariant::BTreeMap { .. } => Ok(btree_map()
            .parse(db, entry)
            .context("failed to parse btree map layout")?),
        _ => {
            todo!(
                "Map variant `{variant:?}` not yet implemented: {}",
                entry.location(db)
            )
        }
    }
}

/// Resolve smart pointer type layout from DWARF
fn resolve_smart_ptr_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    variant: SmartPtrVariant,
) -> Result<SmartPtrLayout> {
    let inner_type = match variant {
        // `Box` is output as a pointer_type and has the type in the entry itself
        SmartPtrVariant::Box => resolve_entry_type_shallow(db, entry)
            .context("Failed to resolve inner type for smart pointer")?,
        // `Rc` and `Arc` are output as struct types, with generic type parameters
        SmartPtrVariant::Arc
        | SmartPtrVariant::Rc
        | SmartPtrVariant::Mutex
        | SmartPtrVariant::Cell
        | SmartPtrVariant::RefCell
        | SmartPtrVariant::UnsafeCell => {
            let type_entry = entry
                .get_generic_type_entry(db, "T")
                .context("could not find inner type")?;

            shallow_resolve_type(db, type_entry).context("failed to resolve the inner type")?
        }
        _ => {
            todo!(
                "{}",
                entry.format_with_location(
                    db,
                    format!("Smart pointer variant `{variant:?}` not yet implemented")
                )
            )
        }
    };

    let (inner_ptr_offset, data_ptr_offset) = match variant {
        SmartPtrVariant::Box => (0, 0), // Box has no offset, it's just a pointer

        SmartPtrVariant::Mutex | SmartPtrVariant::RefCell | SmartPtrVariant::Cell => {
            // Mutex.data -> UnsafeCell<T>
            let mut inner_offset = 0;

            let inner_name = match variant {
                SmartPtrVariant::Mutex => "data",
                SmartPtrVariant::RefCell | SmartPtrVariant::Cell => "value",
                _ => unreachable!(),
            };

            let data = entry.get_member(db, inner_name)?;

            inner_offset += data.udata_attr(db, gimli::DW_AT_data_member_location)?;

            let unsafe_cell_entry = data.get_referenced_entry(db, gimli::DW_AT_type)?;

            // UnsafeCell.value -> T
            inner_offset += unsafe_cell_entry
                .get_udata_member_attribute(db, "value", gimli::DW_AT_data_member_location)
                .context("UnsafeCell value offset is not a valid udata")?;

            (inner_offset, 0)
        }

        SmartPtrVariant::UnsafeCell => {
            // UnsafeCell.value -> T
            let inner_offset = entry
                .get_udata_member_attribute(db, "value", gimli::DW_AT_data_member_location)
                .context("UnsafeCell value offset is not a valid udata")?;

            (inner_offset, 0)
        }
        SmartPtrVariant::Rc | SmartPtrVariant::Arc => {
            // Arc.ptr -> NonNull<ArcInner<T>>

            let mut inner_offset = 0;

            let ptr = entry.get_member(db, "ptr")?;

            inner_offset += ptr
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not find ptr offset")?;

            let nonnull_entry = ptr.get_referenced_entry(db, gimli::DW_AT_type)?;

            // NonNull.pointer -> * ArcInner

            let pointer = nonnull_entry.get_member(db, "pointer")?;

            inner_offset += pointer
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not find pointer offset")?;

            let arcinner_pointer = pointer.get_referenced_entry(db, gimli::DW_AT_type)?;

            // pointer type that needs to be dereferenced to get the inner type
            // then we have _another_ offset to get the data

            let arc_inner = arcinner_pointer.get_referenced_entry(db, gimli::DW_AT_type)?;

            // within {Arc,Rc}Inner, we need to find the actual data pointer offset
            let name = match variant {
                SmartPtrVariant::Arc => "data",
                SmartPtrVariant::Rc => "value",
                _ => unreachable!(),
            };
            let data_ptr_offset = arc_inner
                .get_udata_member_attribute(db, name, gimli::DW_AT_data_member_location)
                .context("data offset is not a valid udata")?;

            (inner_offset, data_ptr_offset)
        }
        _ => {
            todo!(
                "{}",
                entry.format_with_location(
                    db,
                    format!("Smart pointer variant `{variant:?}` not yet implemented")
                )
            )
        }
    };

    Ok(SmartPtrLayout {
        variant,
        inner_type: Arc::new(inner_type),
        inner_ptr_offset,
        data_ptr_offset,
    })
}

fn resolve_tuple_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TupleLayout> {
    let mut elements = Vec::new();
    let size = entry
        .udata_attr(db, gimli::DW_AT_byte_size)
        .context("could not get size for tuple type")?;

    for child in entry.children(db)? {
        let offset = child
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .context("could not get data member location for tuple element")?;
        let ty = resolve_entry_type(db, child)?;
        elements.push((offset, Arc::new(ty)));
    }

    Ok(TupleLayout { elements, size })
}

fn resolve_primitive_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    def: &PrimitiveLayout,
) -> Result<PrimitiveLayout> {
    let def = match def {
        // these "scalar" types are already fully resolved
        PrimitiveLayout::Int(_)
        | PrimitiveLayout::Bool(_)
        | PrimitiveLayout::Char(_)
        | PrimitiveLayout::Float(_)
        | PrimitiveLayout::Never(_)
        | PrimitiveLayout::Str(_)
        | PrimitiveLayout::UnsignedInt(_)
        | PrimitiveLayout::Unit(_) => def.clone(),

        // these types need to be resolved further
        PrimitiveLayout::StrSlice(_) => {
            let data_ptr_offset = entry
                .get_udata_member_attribute(db, "data_ptr", gimli::DW_AT_data_member_location)
                .context("could not find data_ptr")?;

            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not find length")?;

            PrimitiveLayout::StrSlice(StrSliceLayout {
                data_ptr_offset,
                length_offset,
            })
        }
        PrimitiveLayout::Slice(_) => {
            // slices have two members: data_ptr and length
            // the former also specifies the type of the slice
            let data_ptr = entry
                .get_member(db, "data_ptr")
                .context("could not find data_ptr for slice")?;

            let data_ptr_offset = data_ptr
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not get data_ptr offset for slice")?;

            let data_ptr_type_entry = data_ptr
                .get_referenced_entry(db, gimli::DW_AT_type)
                .context("could not get type for data_ptr")?;

            // data_entry is of type `*T` for slice `&[T]`, so we'll deference once more.
            let element_type = resolve_entry_type(db, data_ptr_type_entry)
                .context("could not resolve element type for slice")?;

            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not find length for slice")?;

            PrimitiveLayout::Slice(SliceLayout {
                element_type: Arc::new(element_type),
                data_ptr_offset,
                length_offset,
            })
        }
        PrimitiveLayout::Array(array_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveLayout::Array(ArrayLayout {
                element_type: Arc::new(inner),
                length: array_def.length,
            })
        }
        PrimitiveLayout::Function(_) => {
            // to resolve a function, type, we need to get the return type and argument types
            let return_type = Arc::new(resolve_entry_type(db, entry)?);
            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type(db, c).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;

            PrimitiveLayout::Function(FunctionLayout {
                return_type,
                arg_types,
            })
        }
        PrimitiveLayout::Pointer(pointer_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveLayout::Pointer(PointerLayout {
                mutable: pointer_def.mutable,
                pointed_type: Arc::new(inner),
            })
        }
        PrimitiveLayout::Reference(reference_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveLayout::Reference(ReferenceLayout {
                mutable: reference_def.mutable,
                pointed_type: Arc::new(inner),
            })
        }

        PrimitiveLayout::Tuple(_) => PrimitiveLayout::Tuple(resolve_tuple_type(db, entry)?),
    };

    Ok(def)
}

/// Resolves a type entry to a `Def` _if_ the target entry is one of the
/// support "builtin" types -- these are types that we manually resolve rather
/// than relying on the DWARF info to do so.
fn resolve_as_builtin_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<Option<TypeLayout>> {
    // when detecting builtin types, we look up the typename, which contains
    // our best effort at parsing the type definition based on the name
    // e.g. we detect `alloc::vec::Vec<u8>` as `VecDef` with
    // the primitive `u8` as the element type.

    // From there, we need to full resolve the type the DIE entry
    // (e.g. finding where the pointer + length for the Vec are stored)
    let Some(typename) = get_die_typename(db, entry) else {
        tracing::warn!(
            "no name found for entry: {} at offset {}",
            entry.print(db),
            entry.die_offset(db).0
        );
        return Ok(None);
    };

    tracing::info!(
        "resolve_as_builtin_type: checking typename: {typename} {:#?} {}",
        typename.typedef,
        entry.location(db)
    );

    match &typename.typedef {
        TypeLayout::Primitive(primitive_def) => {
            resolve_primitive_type(db, entry, primitive_def).map(|p| Some(TypeLayout::Primitive(p)))
        }
        TypeLayout::Std(std_def) => {
            // For std types, we need to do additional resolution based on DWARF
            // to get layout information, but we can use the parsed generic types
            match std_def {
                StdLayout::Option(_) => {
                    // Use the parsed Option type but resolve the actual layout
                    let option_def = resolve_option_type(db, entry)?;
                    Ok(Some(TypeLayout::Std(StdLayout::Option(option_def))))
                }
                StdLayout::Vec(_) => {
                    // For Vec, we need to resolve the actual layout from DWARF
                    Ok(vec()
                        .parse(db, entry)
                        .map(|v| Some(TypeLayout::Std(StdLayout::Vec(v))))?)
                }
                StdLayout::String(_) => {
                    // String has a known layout similar to Vec
                    resolve_string_type(db, entry)
                        .map(|s| Some(TypeLayout::Std(StdLayout::String(s))))
                }
                StdLayout::Map(map) => {
                    // HashMap/BTreeMap need layout resolution
                    resolve_map_type(db, entry, map.variant.clone())
                        .map(|m| Some(TypeLayout::Std(StdLayout::Map(m))))
                }
                StdLayout::Result(_) => {
                    // Result types need layout resolution
                    Ok(Some(TypeLayout::Std(StdLayout::Result(
                        resolve_result_type(db, entry)?,
                    ))))
                }
                StdLayout::SmartPtr(s) => {
                    // Smart pointers like Box, Rc, Arc need layout resolution
                    resolve_smart_ptr_type(db, entry, s.variant)
                        .map(|s| Some(TypeLayout::Std(StdLayout::SmartPtr(s))))
                }
            }
        }
        TypeLayout::Struct(_) => {
            // For custom structs, we don't handle them as builtins
            // They'll be resolved through the normal struct resolution path
            Ok(None)
        }
        TypeLayout::Enum(_) => {
            // For custom enums, we don't handle them as builtins
            Ok(None)
        }
        TypeLayout::CEnum(_) => {
            // C enums are handled as custom enums, not builtins
            Ok(None)
        }
        TypeLayout::Alias(_) => {
            // Aliases should be resolved through normal resolution
            Ok(None)
        }
        TypeLayout::Other { name: _ } => {
            // Unknown types are not builtins
            Ok(None)
        }
    }
}

/// "Shallow" resolve a type -- if it's a primitive value, then
/// we'll return that directly. Otherwise, return an alias to some other
/// type entry (if we can find it).
#[salsa::tracked]
pub fn shallow_resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeLayout> {
    Ok(
        if let Some(builtin_ty) = resolve_as_builtin_type(db, entry)? {
            // we have a builtin type -- use it
            tracing::debug!("builtin: {builtin_ty:?}");
            builtin_ty
        } else {
            TypeLayout::Alias(UnresolvedType {
                cu_offset: entry
                    .cu_offset(db)
                    .as_debug_info_offset()
                    .map_or(0, |o| o.0),
                die_offset: entry.die_offset(db).0,
            })
        },
    )
}

fn resolve_enum_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<EnumLayout> {
    Ok(enum_def().parse(db, entry)?)
}

fn resolve_option_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<OptionLayout> {
    Ok(option_def().parse(db, entry)?)
}

fn resolve_result_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<ResultLayout> {
    Ok(result_def().parse(db, entry)?)
}

fn resolve_struct_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<StructLayout> {
    // let index = crate::index::build_index(db);
    // let type_name = index.data(db).die_to_type_name.get(&entry).copied();
    // get some basic info
    let name = entry.name(db)?;
    let size = entry.udata_attr(db, gimli::DW_AT_byte_size)?;
    let alignment = entry.udata_attr(db, gimli::DW_AT_alignment)?;

    // iterate children to get fields
    let mut fields = vec![];
    for child in entry.children(db)? {
        if child.tag(db) != gimli::DW_TAG_member {
            tracing::debug!("skipping non-member entry: {}", child.print(db));
            continue;
        }
        let field_name = child.name(db)?;
        let offset = child.udata_attr(db, gimli::DW_AT_data_member_location)?;

        let type_entry = child.get_referenced_entry(db, gimli::DW_AT_type)?;
        let ty = shallow_resolve_type(db, type_entry)?;

        fields.push(StructField {
            name: field_name,
            offset: offset as usize,
            ty: Arc::new(ty),
        });
    }
    Ok(StructLayout {
        name,
        fields,
        size: size as usize,
        alignment: alignment as usize,
    })
}

/// Fully resolve a type from a DWARF DIE entry
#[salsa::tracked]
pub fn resolve_type_offset<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeLayout> {
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
        gimli::DW_TAG_array_type => {
            let array_type = resolve_entry_type(db, entry)?;
            let children = entry.children(db)?;
            let subrange = children
                .iter()
                .find(|c| c.tag(db) == gimli::DW_TAG_subrange_type)
                .ok_or_else(|| {
                    entry.format_with_location(db, "array type missing subrange information")
                })?;
            let count = subrange.udata_attr(db, gimli::DW_AT_count)?;
            TypeLayout::Primitive(PrimitiveLayout::Array(ArrayLayout {
                element_type: Arc::new(array_type),
                length: count as usize,
            }))
        }
        gimli::DW_TAG_structure_type => {
            // rustc uses `structure_type` for a bunch of things.
            // we need to do a little investigation to figure out what it actually is
            let is_enum = entry
                .children(db)?
                .iter()
                .any(|c| c.tag(db) == gimli::DW_TAG_variant_part);

            if is_enum {
                TypeLayout::Enum(resolve_enum_type(db, entry)?)
            } else {
                // we have a struct type and it's _not_ a builtin -- we'll handle it now
                TypeLayout::Struct(resolve_struct_type(db, entry)?)
            }
        }
        gimli::DW_TAG_subroutine_type => {
            let return_type = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .map_or_else(
                    |_| {
                        // we'll ignore errors -- subroutines with no type field
                        // have no return type
                        Ok(Arc::new(TypeLayout::Primitive(PrimitiveLayout::Unit(
                            UnitLayout,
                        ))))
                    },
                    |ty| resolve_entry_type(db, ty).map(Arc::new),
                )?;

            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type(db, c).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;

            TypeLayout::Primitive(PrimitiveLayout::Function(FunctionLayout {
                return_type,
                arg_types,
            }))
        }
        gimli::DW_TAG_enumeration_type => TypeLayout::CEnum(c_enum_def().parse(db, entry)?),
        t => {
            return Err(entry
                .format_with_location(db, format!("unsupported type: {t}"))
                .into());
        }
    })
}

/// Ensure that a type is fully resolved, including resolving any aliases
/// or references to other types. This is useful for ensuring that the type
/// is ready for use in contexts where a complete type definition is required.
pub fn fully_resolve_type<'db>(
    db: &'db dyn Db,
    file: DebugFile,
    typedef: &TypeLayout,
) -> Result<TypeLayout> {
    let res = match typedef.clone() {
        TypeLayout::Primitive(prim) => {
            use PrimitiveLayout::*;
            let prim = match prim {
                Array(ArrayLayout {
                    element_type,
                    length,
                }) => {
                    let element_type = fully_resolve_type(db, file, element_type.as_ref())?;
                    Array(ArrayLayout {
                        element_type: Arc::new(element_type),
                        length,
                    })
                }
                Function(FunctionLayout {
                    return_type,
                    arg_types,
                }) => {
                    let return_type = Arc::new(fully_resolve_type(db, file, return_type.as_ref())?);
                    let arg_types = arg_types
                        .into_iter()
                        .map(|ty| fully_resolve_type(db, file, ty.as_ref()))
                        .collect::<Result<Vec<_>>>()?;
                    Function(FunctionLayout {
                        return_type,
                        arg_types: arg_types.into_iter().map(Arc::new).collect(),
                    })
                }
                Pointer(PointerLayout {
                    mutable,
                    pointed_type,
                }) => {
                    let pointed_type = fully_resolve_type(db, file, pointed_type.as_ref())?;
                    Pointer(PointerLayout {
                        mutable,
                        pointed_type: Arc::new(pointed_type),
                    })
                }
                Reference(ReferenceLayout {
                    mutable,
                    pointed_type,
                }) => {
                    let pointed_type = fully_resolve_type(db, file, pointed_type.as_ref())?;
                    Reference(ReferenceLayout {
                        mutable,
                        pointed_type: Arc::new(pointed_type),
                    })
                }
                Slice(SliceLayout {
                    element_type,
                    data_ptr_offset,
                    length_offset,
                }) => {
                    let element_type = fully_resolve_type(db, file, element_type.as_ref())?;
                    Slice(SliceLayout {
                        element_type: Arc::new(element_type),
                        data_ptr_offset,
                        length_offset,
                    })
                }
                Tuple(TupleLayout { elements, size }) => {
                    let elements = elements
                        .into_iter()
                        .map(|(offset, ty)| {
                            fully_resolve_type(db, file, ty.as_ref())
                                .map(|ty| (offset, Arc::new(ty)))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Tuple(TupleLayout { elements, size })
                }
                p @ (Bool(_) | Char(_) | Float(_) | Int(_) | Never(_) | Str(_) | StrSlice(_)
                | Unit(_) | UnsignedInt(_)) => p,
            };
            TypeLayout::Primitive(prim)
        }
        TypeLayout::Std(std_def) => {
            use StdLayout::*;
            let std_def = match std_def {
                SmartPtr(SmartPtrLayout {
                    inner_type,
                    inner_ptr_offset,
                    data_ptr_offset,
                    variant,
                }) => {
                    let inner_type = fully_resolve_type(db, file, inner_type.as_ref())?;
                    SmartPtr(SmartPtrLayout {
                        inner_type: Arc::new(inner_type),
                        inner_ptr_offset,
                        data_ptr_offset,
                        variant,
                    })
                }
                Map(MapLayout {
                    key_type,
                    value_type,
                    variant,
                }) => {
                    let key_type = fully_resolve_type(db, file, key_type.as_ref())?;
                    let value_type = fully_resolve_type(db, file, value_type.as_ref())?;
                    Map(MapLayout {
                        key_type: Arc::new(key_type),
                        value_type: Arc::new(value_type),
                        variant,
                    })
                }
                Option(OptionLayout {
                    name,
                    discriminant,
                    some_type,
                    some_offset,
                    size,
                }) => {
                    let some_type = fully_resolve_type(db, file, some_type.as_ref())?;

                    StdLayout::Option(OptionLayout {
                        name,
                        discriminant,
                        some_type: Arc::new(some_type),
                        some_offset,
                        size,
                    })
                }
                Result(ResultLayout {
                    name,
                    discriminant,
                    ok_type,
                    err_type,
                    size,
                }) => {
                    let ok_type = fully_resolve_type(db, file, ok_type.as_ref())?;
                    let err_type = fully_resolve_type(db, file, err_type.as_ref())?;

                    StdLayout::Result(ResultLayout {
                        name,
                        discriminant,
                        ok_type: Arc::new(ok_type),
                        err_type: Arc::new(err_type),
                        size,
                    })
                }
                String(string_def) => String(string_def),
                Vec(VecLayout {
                    length_offset,
                    data_ptr_offset,
                    inner_type,
                }) => {
                    let inner_type = fully_resolve_type(db, file, inner_type.as_ref())?;
                    Vec(VecLayout {
                        length_offset,
                        data_ptr_offset,
                        inner_type: Arc::new(inner_type),
                    })
                }
            };
            TypeLayout::Std(std_def)
        }
        TypeLayout::Struct(StructLayout {
            name,
            size,
            alignment,
            fields,
        }) => {
            // For structs, we need to resolve each field's type
            let fields = fields
                .into_iter()
                .map(|field| {
                    let ty = fully_resolve_type(db, file, field.ty.as_ref())?;
                    Ok(StructField {
                        name: field.name,
                        offset: field.offset,
                        ty: Arc::new(ty),
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            TypeLayout::Struct(StructLayout {
                name,
                size,
                alignment,
                fields,
            })
        }
        TypeLayout::Enum(EnumLayout {
            name,
            variants,
            size,
            discriminant,
        }) => {
            // For enums, we need to resolve each variant's type
            let variants = variants
                .into_iter()
                .map(
                    |EnumVariantLayout {
                         name,
                         discriminant,
                         layout,
                     }| {
                        let layout = fully_resolve_type(db, file, layout.as_ref())?;
                        Ok(EnumVariantLayout {
                            name,
                            discriminant,
                            layout: Arc::new(layout),
                        })
                    },
                )
                .collect::<Result<_>>()?;

            TypeLayout::Enum(EnumLayout {
                name,
                discriminant,
                variants,
                size,
            })
        }
        TypeLayout::CEnum(c_enum_def) => {
            // C enums don't have nested types to resolve
            TypeLayout::CEnum(c_enum_def)
        }
        TypeLayout::Alias(UnresolvedType {
            cu_offset,
            die_offset,
        }) => {
            let die_offset = UnitOffset(die_offset);
            let cu_offset = gimli::UnitSectionOffset::from(DebugInfoOffset(cu_offset));
            let die = Die::new(db, file, cu_offset, die_offset);
            resolve_type_offset(db, die)
                .with_context(|| die.format_with_location(db, "Failed to resolve alias type"))?
        }
        TypeLayout::Other { name } => {
            // Other types are not fully resolved, return as is
            TypeLayout::Other { name }
        }
    };

    Ok(res)
}

#[cfg(test)]
mod test {
    use tracing_subscriber::EnvFilter;

    use crate::{DebugDb, DebugInfo, dwarf::resolve_function_variables};

    #[test]
    fn test_std_type_detection() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .try_init();

        let db = DebugDb::new();

        // Create a test binary that should have std types
        let test_binary_path = create_test_binary();

        tracing::info!("=== Testing std type detection ===");
        tracing::info!("Using binary: {test_binary_path}");

        let debug_info =
            DebugInfo::new(&db, &test_binary_path).expect("Failed to create debug info");
        // For now, just test that we can create the debug info
        // You can add more specific tests here as you implement the functionality
        tracing::info!("DebugInfo created successfully");

        let function_name = "test_fn";
        let (f, _debug_file) =
            crate::index::find_closest_function(debug_info.db, debug_info.binary, function_name)
                .expect("Failed to find function");

        let (_debug_file, fie) = crate::index::debug_index(&db, debug_info.binary)
            .get_function(&db, &f)
            .expect("Failed to find function in debug index");

        let function_die = fie.specification_die.unwrap_or(fie.declaration_die);
        let params = resolve_function_variables(&db, function_die)
            .expect("Failed to resolve function variables");
        assert_eq!(
            params.params(&db).len(),
            3,
            "Expected 3 parameters in test_fn"
        );

        // Check if we can resolve the types of the parameters
        for param in params.params(&db) {
            let ty = param.ty(&db);

            insta::with_settings!({
                prepend_module_to_snapshot => false,
                filters => vec![
                (r"tv_sec: [0-9]+", "tv_sec: [ts]"),
                (r"tv_nsec: [0-9]+", "tv_nsec: [ts]"),
            ]}, {
                salsa::attach(&db, || insta::assert_debug_snapshot!(ty));
            });
        }

        // Test resolving variables if we can find any
        // This is a basic smoke test to make sure the new get_die_name function works
        tracing::info!("DebugInfo appears to be working correctly");
    }

    fn create_test_binary() -> String {
        use std::fs;
        use std::process::Command;

        let test_code = r#"
    use std::collections::HashMap;

    fn test_fn(s: String, v: Vec<i32>, map: HashMap<String, i32>) {
        println!("String: {s}, Vec: {v:?}, Map: {map:?}");
    }

    fn main() {
        let s = String::from("hello");
        let mut v: Vec<i32> = vec![1, 2, 3];
        let mut map: HashMap<String, i32> = HashMap::new();
        map.insert("key".to_string(), 42);

        test_fn(s, v, map);
    }
    "#;

        let temp_dir = std::env::temp_dir().join("rudy_db_test");
        fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

        let src_file = temp_dir.join("main.rs");
        fs::write(&src_file, test_code).expect("Failed to write test code");

        let binary_path = temp_dir.join("test_binary");

        let output = Command::new("rustc")
            .args(&[
                "-g", // Include debug info
                "-C",
                "split-debuginfo=unpacked", // Use unpacked split debuginfo
                "-o",
                binary_path.to_str().unwrap(),
                src_file.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to compile test binary");

        if !output.status.success() {
            panic!(
                "Failed to compile test binary: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        binary_path.to_string_lossy().to_string()
    }
}
