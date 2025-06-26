//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::index::get_die_typename;
use crate::dwarf::resolution::parser::{Parser, child_field_type, vec_parser};
use crate::file::DebugFile;
use rust_types::*;

use anyhow::Context;
use gimli::{DebugInfoOffset, UnitOffset};

type Result<T> = std::result::Result<T, super::Error>;

/// Resolve the full type for a DIE entry
pub fn resolve_entry_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry.get_unit_ref(db, gimli::DW_AT_type)?;
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_entry_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry.get_unit_ref(db, gimli::DW_AT_type)?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve String type layout from DWARF
fn resolve_string_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<StringDef> {
    // Get the vec field type and parse it as a Vec
    child_field_type("vec")
        .then(vec_parser())
        .parse(db, entry)
        .map(StringDef)
}

/// Resolve Map type layout from DWARF
fn resolve_map_type<'db>(db: &'db dyn Db, entry: Die<'db>, variant: MapVariant) -> Result<MapDef> {
    let key_type = entry
        .get_generic_type_entry(db, "K")
        .context("could not find key type for map")?;
    let key_type =
        Arc::new(shallow_resolve_type(db, key_type).context("could not resolve key type for map")?);
    let value_type = entry
        .get_generic_type_entry(db, "V")
        .context("could not find value type for map")?;
    let value_type = Arc::new(
        shallow_resolve_type(db, value_type).context("could not resolve value type for map")?,
    );

    match variant {
        MapVariant::HashMap { .. } => {
            // first, let's detect what kind of hashmap we're dealing with by inspecting the inner type
            let fully_qualified_type_name = get_die_typename(db, entry)
                .context("could not get fully qualified type name for table")?;

            match fully_qualified_type_name.module.segments[0].as_str() {
                "hashbrown" => {
                    // this is a hashbrown hashmap
                    tracing::debug!(
                        "detected hashbrown hashmap: {}",
                        fully_qualified_type_name.name
                    );
                }
                "std" | "alloc" => {
                    // this is a std or alloc hashmap
                    tracing::debug!(
                        "detected std/alloc hashmap: {}",
                        fully_qualified_type_name.name
                    );

                    // As of Rust 1.52 or so, it seems like hashbrown is used as the
                    // implementation for std::collections::HashMap, so we can treat it as such

                    let base = entry
                        .get_member(db, "base")
                        .context("could not find base field for HashMap")?
                        .get_unit_ref(db, gimli::DW_AT_type)
                        .context("could not get type for base field")?;

                    return resolve_map_type(db, base, variant);
                }
                s => {
                    return Err(entry
                        .format_with_location(db, format!("unexpected hashmap type: {s}"))
                        .into());
                }
            }

            let table = entry
                .get_member(db, "table")
                .context("could not find table field for HashMap")?;
            let mut table_offset = table
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("table offset for HashMap")? as usize;

            let inner_table_type = table
                .get_unit_ref(db, gimli::DW_AT_type)
                .context("could not get type for table field")?;

            // we also need to get the offsets + size of the (k, v) pairs
            // stored in the rawtable
            let kv_type = inner_table_type
                .get_generic_type_entry(db, "T")
                .context("could not get (k, v) type for raw table")?;
            let pair_size = kv_type
                .udata_attr(db, gimli::DW_AT_byte_size)
                .context("could not get (k, v) pair size for raw table")?;
            let key_offset = kv_type
                .get_udata_member_attribute(db, "__0", gimli::DW_AT_data_member_location)
                .context("could not get key offset for raw table")?;
            let value_offset = kv_type
                .get_udata_member_attribute(db, "__1", gimli::DW_AT_data_member_location)
                .context("could not get value offset for raw table")?;

            let member = inner_table_type.get_member(db, "table")?;
            table_offset += member
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not find member offset for HashMap")?
                as usize;

            // hashbrow::raw::RawTableInner type entry
            let raw_type_inner = member
                .get_unit_ref(db, gimli::DW_AT_type)
                .context("could not get type for member field")?;

            // Extract field offsets from RawTableInner
            let bucket_mask_offset = table_offset
                + raw_type_inner.get_udata_member_attribute(
                    db,
                    "bucket_mask",
                    gimli::DW_AT_data_member_location,
                )?;
            let ctrl_offset = table_offset
                + raw_type_inner.get_udata_member_attribute(
                    db,
                    "ctrl",
                    gimli::DW_AT_data_member_location,
                )?;
            let items_offset = table_offset
                + raw_type_inner.get_udata_member_attribute(
                    db,
                    "items",
                    gimli::DW_AT_data_member_location,
                )?;

            Ok(MapDef {
                key_type,
                value_type,
                variant: MapVariant::HashMap {
                    bucket_mask_offset,
                    ctrl_offset,
                    items_offset,
                    pair_size,
                    key_offset,
                    value_offset,
                },
            })
        }
        MapVariant::BTreeMap { .. } => {
            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not get length offset for BTreeMap")?;

            // Try to resolve LeafNode structure from DWARF to get actual offsets
            let (root_offset, root_layout) = resolve_leaf_node_layout(db, entry)?;

            Ok(MapDef {
                key_type,
                value_type,
                variant: MapVariant::BTreeMap {
                    length_offset,
                    root_offset,
                    root_layout,
                },
            })
        }
        _ => {
            todo!(
                "Map variant `{variant:?}` not yet implemented: {}",
                entry.location(db)
            )
        }
    }
}

/// Resolve LeafNode layout from DWARF debug information
fn resolve_leaf_node_layout<'db>(
    db: &'db dyn Db,
    btree_entry: Die<'db>,
) -> Result<(usize, EnumDef)> {
    // From the BTreeMap entry, find the LeafNode type through the Root field
    // BTreeMap -> root: Option<Root<K, V>> -> Root<K, V> -> node: NodeRef<...> -> LeafNode<K, V>

    // Get the root field type
    let root_member = btree_entry
        .get_member(db, "root")
        .context("could not find root member in BTreeMap")?;

    let root_offset = root_member
        .udata_attr(db, gimli::DW_AT_data_member_location)
        .context("could not get root field offset")? as usize;

    // Option<NodeRef<...>
    let root_type = root_member
        .get_unit_ref(db, gimli::DW_AT_type)
        .context("could not get root field type")?;

    // just return the optional noderef definition directly
    Ok((
        root_offset,
        resolve_enum_type(db, root_type).context("could not resolve type for root field")?,
    ))
}

/// Extract array capacity from a DWARF array type
fn extract_array_capacity<'db>(db: &'db dyn Db, array_die: Die<'db>) -> Result<usize> {
    // Look for subrange_type child that contains the array bounds
    let subrange = array_die
        .get_member_by_tag(db, gimli::DW_TAG_subrange_type)
        .context("could not find subrange_type for array")?;

    // Try to get DW_AT_count or DW_AT_upper_bound
    if let Ok(count) = subrange.udata_attr(db, gimli::DW_AT_count) {
        Ok(count)
    } else if let Ok(upper_bound) = subrange.udata_attr(db, gimli::DW_AT_upper_bound) {
        Ok(upper_bound + 1) // upper_bound is inclusive, so add 1
    } else {
        Err(anyhow::anyhow!("Could not determine array capacity").into())
    }
}

/// Resolve smart pointer type layout from DWARF
fn resolve_smart_ptr_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    variant: SmartPtrVariant,
) -> Result<SmartPtrDef> {
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

            let unsafe_cell_entry = data.get_unit_ref(db, gimli::DW_AT_type)?;

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

            let nonnull_entry = ptr.get_unit_ref(db, gimli::DW_AT_type)?;

            // NonNull.pointer -> * ArcInner

            let pointer = nonnull_entry.get_member(db, "pointer")?;

            inner_offset += pointer
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not find pointer offset")?;

            let arcinner_pointer = pointer.get_unit_ref(db, gimli::DW_AT_type)?;

            // pointer type that needs to be dereferenced to get the inner type
            // then we have _another_ offset to get the data

            let arc_inner = arcinner_pointer.get_unit_ref(db, gimli::DW_AT_type)?;

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

    Ok(SmartPtrDef {
        variant,
        inner_type: Arc::new(inner_type),
        inner_ptr_offset,
        data_ptr_offset,
    })
}

fn resolve_tuple_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TupleDef> {
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

    Ok(TupleDef { elements, size })
}

fn resolve_primitive_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    def: &PrimitiveDef,
) -> Result<PrimitiveDef> {
    let def = match def {
        // these "scalar" types are already fully resolved
        PrimitiveDef::Int(_)
        | PrimitiveDef::Bool(_)
        | PrimitiveDef::Char(_)
        | PrimitiveDef::Float(_)
        | PrimitiveDef::Never(_)
        | PrimitiveDef::Str(_)
        | PrimitiveDef::UnsignedInt(_)
        | PrimitiveDef::Unit(_) => def.clone(),

        // these types need to be resolved further
        PrimitiveDef::StrSlice(_) => {
            let data_ptr_offset = entry
                .get_udata_member_attribute(db, "data_ptr", gimli::DW_AT_data_member_location)
                .context("could not find data_ptr")?;

            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not find length")?;

            PrimitiveDef::StrSlice(StrSliceDef {
                data_ptr_offset,
                length_offset,
            })
        }
        PrimitiveDef::Slice(_) => {
            // slices have two members: data_ptr and length
            // the former also specifies the type of the slice
            let data_ptr = entry
                .get_member(db, "data_ptr")
                .context("could not find data_ptr for slice")?;

            let data_ptr_offset = data_ptr
                .udata_attr(db, gimli::DW_AT_data_member_location)
                .context("could not get data_ptr offset for slice")?;

            let data_ptr_type_entry = data_ptr
                .get_unit_ref(db, gimli::DW_AT_type)
                .context("could not get type for data_ptr")?;

            // data_entry is of type `*T` for slice `&[T]`, so we'll deference once more.
            let element_type = resolve_entry_type(db, data_ptr_type_entry)
                .context("could not resolve element type for slice")?;

            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not find length for slice")?;

            PrimitiveDef::Slice(SliceDef {
                element_type: Arc::new(element_type),
                data_ptr_offset,
                length_offset,
            })
        }
        PrimitiveDef::Array(array_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveDef::Array(ArrayDef {
                element_type: Arc::new(inner),
                length: array_def.length,
            })
        }
        PrimitiveDef::Function(_) => {
            // to resolve a function, type, we need to get the return type and argument types
            let return_type = Arc::new(resolve_entry_type(db, entry)?);
            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type(db, c).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;

            PrimitiveDef::Function(FunctionDef {
                return_type,
                arg_types,
            })
        }
        PrimitiveDef::Pointer(pointer_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveDef::Pointer(PointerDef {
                mutable: pointer_def.mutable,
                pointed_type: Arc::new(inner),
            })
        }
        PrimitiveDef::Reference(reference_def) => {
            let inner = resolve_entry_type(db, entry)?;
            PrimitiveDef::Reference(ReferenceDef {
                mutable: reference_def.mutable,
                pointed_type: Arc::new(inner),
            })
        }

        PrimitiveDef::Tuple(_) => PrimitiveDef::Tuple(resolve_tuple_type(db, entry)?),
    };

    Ok(def)
}

/// Resolves a type entry to a `Def` _if_ the target entry is one of the
/// support "builtin" types -- these are types that we manually resolve rather
/// than relying on the DWARF info to do so.
fn resolve_as_builtin_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<Option<TypeDef>> {
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
        TypeDef::Primitive(primitive_def) => {
            resolve_primitive_type(db, entry, primitive_def).map(|p| Some(TypeDef::Primitive(p)))
        }
        TypeDef::Std(std_def) => {
            // For std types, we need to do additional resolution based on DWARF
            // to get layout information, but we can use the parsed generic types
            match std_def {
                StdDef::Option(_) => {
                    // Use the parsed Option type but resolve the actual layout
                    let option_def = resolve_enum_type(db, entry)?;
                    Ok(Some(TypeDef::Std(StdDef::Option(option_def))))
                }
                StdDef::Vec(_) => {
                    // For Vec, we need to resolve the actual layout from DWARF
                    vec_parser()
                        .parse(db, entry)
                        .map(|v| Some(TypeDef::Std(StdDef::Vec(v))))
                }
                StdDef::String(_) => {
                    // String has a known layout similar to Vec
                    resolve_string_type(db, entry).map(|s| Some(TypeDef::Std(StdDef::String(s))))
                }
                StdDef::Map(map) => {
                    // HashMap/BTreeMap need layout resolution
                    resolve_map_type(db, entry, map.variant.clone())
                        .map(|m| Some(TypeDef::Std(StdDef::Map(m))))
                }
                StdDef::Result(_) => {
                    // Result types need layout resolution
                    Ok(Some(TypeDef::Std(StdDef::Result(resolve_enum_type(
                        db, entry,
                    )?))))
                }
                StdDef::SmartPtr(s) => {
                    // Smart pointers like Box, Rc, Arc need layout resolution
                    resolve_smart_ptr_type(db, entry, s.variant)
                        .map(|s| Some(TypeDef::Std(StdDef::SmartPtr(s))))
                }
            }
        }
        TypeDef::Struct(_) => {
            // For custom structs, we don't handle them as builtins
            // They'll be resolved through the normal struct resolution path
            Ok(None)
        }
        TypeDef::Enum(_) => {
            // For custom enums, we don't handle them as builtins
            Ok(None)
        }
        TypeDef::Alias(_) => {
            // Aliases should be resolved through normal resolution
            Ok(None)
        }
        TypeDef::Other { name: _ } => {
            // Unknown types are not builtins
            Ok(None)
        }
    }
}

/// "Shallow" resolve a type -- if it's a primitive value, then
/// we'll return that directly. Otherwise, return an alias to some other
/// type entry (if we can find it).
#[salsa::tracked]
pub fn shallow_resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    Ok(
        if let Some(builtin_ty) = resolve_as_builtin_type(db, entry)? {
            // we have a builtin type -- use it
            tracing::debug!("builtin: {builtin_ty:?}");
            builtin_ty
        } else {
            TypeDef::Alias(TypeRef {
                cu_offset: entry
                    .cu_offset(db)
                    .as_debug_info_offset()
                    .map_or(0, |o| o.0),
                die_offset: entry.die_offset(db).0,
            })
        },
    )
}

fn resolve_enum_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<EnumDef> {
    // general idea of resolving an enum type:
    // 1. get the name of the enum
    // 2. we have one child which is the variant part
    //    which contains the discriminant and the variants,
    // 3. For each variant, we have a structure_type child (to be validate)
    //    and so we can get the type of the variant

    let name = entry.name(db)?;
    tracing::debug!("resolving enum type: {name} {}", entry.print(db));

    let mut variants = vec![];
    let size = entry.udata_attr(db, gimli::DW_AT_byte_size)?;

    // get the variant part
    let variants_entry = entry.get_member_by_tag(db, gimli::DW_TAG_variant_part)?;

    let discriminant = if let Ok(discriminant) = variants_entry.get_unit_ref(db, gimli::DW_AT_discr)
    {
        let offset = variants_entry
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .unwrap_or(0);
        // we have an explicit discriminant
        // resolve it to a type
        let discriminant_type = resolve_entry_type(db, discriminant)?;
        let ty = match discriminant_type {
            TypeDef::Primitive(PrimitiveDef::Int(i)) => DiscriminantType::Int(i),
            TypeDef::Primitive(PrimitiveDef::UnsignedInt(u)) => DiscriminantType::UnsignedInt(u),
            _ => {
                tracing::warn!(
                    "discriminant type is not an integer: {discriminant_type:?} {}",
                    entry.location(db)
                );
                DiscriminantType::Implicit
            }
        };
        Discriminant { ty, offset }
    } else {
        // no explicit discriminant, so we assume it's implicit
        Discriminant {
            ty: DiscriminantType::Implicit,
            offset: 0,
        }
    };

    for variant in variants_entry.children(db)? {
        if variant.tag(db) != gimli::DW_TAG_variant {
            tracing::debug!("skipping non-variant entry: {}", variant.print(db));
            continue;
        }
        tracing::debug!("variant: {}", variant.print(db));

        let discriminant = variant
            .udata_attr(db, gimli::DW_AT_discr_value)
            .unwrap_or(variants.len()) as u64;

        // should have a single member
        let member = variant
            .get_member_by_tag(db, gimli::DW_TAG_member)
            .context("variant part should have a single member")?;

        let variant_type = Arc::new(resolve_entry_type_shallow(db, member)?);
        variants.push(EnumVariant {
            name: member.name(db)?,
            discriminant,
            layout: variant_type,
        });
    }

    Ok(EnumDef {
        name,
        variants,
        size,
        discriminant,
    })
}

fn resolve_struct_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<StructDef> {
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

        let type_entry = child.get_unit_ref(db, gimli::DW_AT_type)?;
        let ty = shallow_resolve_type(db, type_entry)?;

        fields.push(StructField {
            name: field_name,
            offset: offset as usize,
            ty: Arc::new(ty),
        });
    }
    Ok(StructDef {
        name,
        fields,
        size: size as usize,
        alignment: alignment as usize,
    })
}

/// Fully resolve a type from a DWARF DIE entry
#[salsa::tracked]
pub fn resolve_type_offset<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
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
            TypeDef::Primitive(PrimitiveDef::Array(ArrayDef {
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
                TypeDef::Enum(resolve_enum_type(db, entry)?)
            } else {
                // we have a struct type and it's _not_ a builtin -- we'll handle it now
                TypeDef::Struct(resolve_struct_type(db, entry)?)
            }
        }
        gimli::DW_TAG_subroutine_type => {
            let return_type = entry.get_unit_ref(db, gimli::DW_AT_type).map_or_else(
                |_| {
                    // we'll ignore errors -- subroutines with no type field
                    // have no return type
                    Ok(Arc::new(TypeDef::Primitive(PrimitiveDef::Unit(UnitDef))))
                },
                |ty| resolve_entry_type(db, ty).map(Arc::new),
            )?;

            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type(db, c).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;

            TypeDef::Primitive(PrimitiveDef::Function(FunctionDef {
                return_type,
                arg_types,
            }))
        }
        // gimli::DW_TAG_enumeration_type => {
        //     todo!(
        //         "enumeration type not yet implemented: {}",
        //         entry.format_with_location(db, "enumeration type")
        //     );
        // }
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
    typedef: &TypeDef,
) -> Result<TypeDef> {
    let res = match typedef.clone() {
        TypeDef::Primitive(prim) => {
            use PrimitiveDef::*;
            let prim = match prim {
                Array(ArrayDef {
                    element_type,
                    length,
                }) => {
                    let element_type = fully_resolve_type(db, file, element_type.as_ref())?;
                    Array(ArrayDef {
                        element_type: Arc::new(element_type),
                        length,
                    })
                }
                Function(FunctionDef {
                    return_type,
                    arg_types,
                }) => {
                    let return_type = Arc::new(fully_resolve_type(db, file, return_type.as_ref())?);
                    let arg_types = arg_types
                        .into_iter()
                        .map(|ty| fully_resolve_type(db, file, ty.as_ref()))
                        .collect::<Result<Vec<_>>>()?;
                    Function(FunctionDef {
                        return_type,
                        arg_types: arg_types.into_iter().map(Arc::new).collect(),
                    })
                }
                Pointer(PointerDef {
                    mutable,
                    pointed_type,
                }) => {
                    let pointed_type = fully_resolve_type(db, file, pointed_type.as_ref())?;
                    Pointer(PointerDef {
                        mutable,
                        pointed_type: Arc::new(pointed_type),
                    })
                }
                Reference(ReferenceDef {
                    mutable,
                    pointed_type,
                }) => {
                    let pointed_type = fully_resolve_type(db, file, pointed_type.as_ref())?;
                    Reference(ReferenceDef {
                        mutable,
                        pointed_type: Arc::new(pointed_type),
                    })
                }
                Slice(SliceDef {
                    element_type,
                    data_ptr_offset,
                    length_offset,
                }) => {
                    let element_type = fully_resolve_type(db, file, element_type.as_ref())?;
                    Slice(SliceDef {
                        element_type: Arc::new(element_type),
                        data_ptr_offset,
                        length_offset,
                    })
                }
                Tuple(TupleDef { elements, size }) => {
                    let elements = elements
                        .into_iter()
                        .map(|(offset, ty)| {
                            fully_resolve_type(db, file, ty.as_ref())
                                .map(|ty| (offset, Arc::new(ty)))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Tuple(TupleDef { elements, size })
                }
                p @ (Bool(_) | Char(_) | Float(_) | Int(_) | Never(_) | Str(_) | StrSlice(_)
                | Unit(_) | UnsignedInt(_)) => p,
            };
            TypeDef::Primitive(prim)
        }
        TypeDef::Std(std_def) => {
            use StdDef::*;
            let std_def = match std_def {
                SmartPtr(SmartPtrDef {
                    inner_type,
                    inner_ptr_offset,
                    data_ptr_offset,
                    variant,
                }) => {
                    let inner_type = fully_resolve_type(db, file, inner_type.as_ref())?;
                    SmartPtr(SmartPtrDef {
                        inner_type: Arc::new(inner_type),
                        inner_ptr_offset,
                        data_ptr_offset,
                        variant,
                    })
                }
                Map(MapDef {
                    key_type,
                    value_type,
                    variant,
                }) => {
                    let key_type = fully_resolve_type(db, file, key_type.as_ref())?;
                    let value_type = fully_resolve_type(db, file, value_type.as_ref())?;
                    Map(MapDef {
                        key_type: Arc::new(key_type),
                        value_type: Arc::new(value_type),
                        variant,
                    })
                }
                Option(EnumDef {
                    name,
                    discriminant,
                    variants,
                    size,
                }) => {
                    // For enums, we need to resolve each variant's type
                    let variants = variants
                        .into_iter()
                        .map(
                            |EnumVariant {
                                 name,
                                 discriminant,
                                 layout,
                             }| {
                                let layout = fully_resolve_type(db, file, layout.as_ref())?;
                                Ok(EnumVariant {
                                    name,
                                    discriminant,
                                    layout: Arc::new(layout),
                                })
                            },
                        )
                        .collect::<anyhow::Result<_>>()?;

                    StdDef::Option(EnumDef {
                        name,
                        discriminant,
                        variants,
                        size,
                    })
                }
                Result(EnumDef {
                    name,
                    discriminant,
                    variants,
                    size,
                }) => {
                    // For enums, we need to resolve each variant's type
                    let variants = variants
                        .into_iter()
                        .map(
                            |EnumVariant {
                                 name,
                                 discriminant,
                                 layout,
                             }| {
                                let layout = fully_resolve_type(db, file, layout.as_ref())?;
                                Ok(EnumVariant {
                                    name,
                                    discriminant,
                                    layout: Arc::new(layout),
                                })
                            },
                        )
                        .collect::<anyhow::Result<_>>()?;

                    StdDef::Result(EnumDef {
                        name,
                        discriminant,
                        variants,
                        size,
                    })
                }
                String(string_def) => String(string_def),
                Vec(VecDef {
                    length_offset,
                    data_ptr_offset,
                    inner_type,
                }) => {
                    let inner_type = fully_resolve_type(db, file, inner_type.as_ref())?;
                    Vec(VecDef {
                        length_offset,
                        data_ptr_offset,
                        inner_type: Arc::new(inner_type),
                    })
                }
            };
            TypeDef::Std(std_def)
        }
        TypeDef::Struct(StructDef {
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

            TypeDef::Struct(StructDef {
                name,
                size,
                alignment,
                fields,
            })
        }
        TypeDef::Enum(EnumDef {
            name,
            variants,
            size,
            discriminant,
        }) => {
            // For enums, we need to resolve each variant's type
            let variants = variants
                .into_iter()
                .map(
                    |EnumVariant {
                         name,
                         discriminant,
                         layout,
                     }| {
                        let layout = fully_resolve_type(db, file, layout.as_ref())?;
                        Ok(EnumVariant {
                            name,
                            discriminant,
                            layout: Arc::new(layout),
                        })
                    },
                )
                .collect::<Result<_>>()?;

            TypeDef::Enum(EnumDef {
                name,
                discriminant,
                variants,
                size,
            })
        }
        TypeDef::Alias(TypeRef {
            cu_offset,
            die_offset,
        }) => {
            let die_offset = UnitOffset(die_offset);
            let cu_offset = gimli::UnitSectionOffset::from(DebugInfoOffset(cu_offset));
            let die = Die::new(db, file, cu_offset, die_offset);
            resolve_type_offset(db, die).context("Failed to resolve alias type")?
        }
        TypeDef::Other { name } => {
            // Other types are not fully resolved, return as is
            TypeDef::Other { name }
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

        let temp_dir = std::env::temp_dir().join("rust_debuginfo_test");
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
