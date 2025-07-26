//! Type resolution from DWARF debugging information

use anyhow::Context;
use rudy_types::*;

use crate::{
    parser::{
        btreemap::btree_map,
        children::parse_children,
        enums::{c_enum_def, enum_def},
        hashmap::hashbrown_map,
        option::option_def,
        primitives::{entry_type, member, resolved_generic},
        result::result_def,
        vec::vec,
        Parser,
    },
    types::{get_die_typename, DieTypeDefinition, TypeIndexEntry},
    DebugFile, Die, DwarfDb,
};

type Result<T> = std::result::Result<T, crate::Error>;

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_entry_type_shallow(db: &dyn DwarfDb, entry: Die) -> Result<DieTypeDefinition> {
    let type_entry = entry.get_referenced_entry(db, gimli::DW_AT_type)?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve String type layout from DWARF
fn resolve_string_type(db: &dyn DwarfDb, entry: Die) -> Result<StringLayout<Die>> {
    // Get the vec field type and parse it as a Vec
    Ok(member("vec")
        .then(entry_type())
        .then(vec())
        .parse(db, entry)
        .map(StringLayout)?)
}

/// Resolve Map type layout from DWARF
fn resolve_map_type(db: &dyn DwarfDb, entry: Die, variant: MapVariant) -> Result<MapLayout<Die>> {
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
            unimplemented!(
                "Map variant `{variant:?}` not yet implemented: {}",
                entry.location(db)
            )
        }
    }
}

/// Resolve smart pointer type layout from DWARF
fn resolve_smart_ptr_type(
    db: &dyn DwarfDb,
    entry: Die,
    variant: SmartPtrVariant,
) -> Result<SmartPtrLayout<Die>> {
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
            unimplemented!(
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
            unimplemented!(
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
        inner_type,
        inner_ptr_offset,
        data_ptr_offset,
    })
}

fn resolve_tuple_type(db: &dyn DwarfDb, entry: Die) -> Result<TupleLayout<Die>> {
    let mut elements = Vec::new();
    let size = entry
        .udata_attr(db, gimli::DW_AT_byte_size)
        .context("could not get size for tuple type")?;

    for child in entry.children(db)? {
        let offset = child
            .udata_attr(db, gimli::DW_AT_data_member_location)
            .context("could not get data member location for tuple element")?;
        let ty = resolve_entry_type_shallow(db, child)?;
        elements.push((offset, ty));
    }

    Ok(TupleLayout { elements, size })
}

fn resolve_primitive_type<L: Location + Clone>(
    db: &dyn DwarfDb,
    entry: Die,
    def: &PrimitiveLayout<L>,
) -> Result<PrimitiveLayout<Die>> {
    let layout = match def {
        // these "scalar" types are already fully resolved
        // and have no dependency on the location info
        PrimitiveLayout::Int(i) => PrimitiveLayout::Int(*i),
        PrimitiveLayout::Bool(_) => PrimitiveLayout::Bool(()),
        PrimitiveLayout::Char(_) => PrimitiveLayout::Char(()),
        PrimitiveLayout::Float(f) => PrimitiveLayout::Float(*f),
        PrimitiveLayout::Never(_) => PrimitiveLayout::Never(()),
        PrimitiveLayout::Str(_) => PrimitiveLayout::Str(()),
        PrimitiveLayout::UnsignedInt(u) => PrimitiveLayout::UnsignedInt(*u),
        PrimitiveLayout::Unit(_) => PrimitiveLayout::Unit(UnitLayout),

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
            let element_type = resolve_entry_type_shallow(db, data_ptr_type_entry)
                .context("could not resolve element type for slice")?;

            let length_offset = entry
                .get_udata_member_attribute(db, "length", gimli::DW_AT_data_member_location)
                .context("could not find length for slice")?;

            PrimitiveLayout::Slice(SliceLayout {
                element_type,
                data_ptr_offset,
                length_offset,
            })
        }
        PrimitiveLayout::Array(array_def) => {
            let element_type = resolve_entry_type_shallow(db, entry)?;
            PrimitiveLayout::Array(ArrayLayout {
                element_type,
                length: array_def.length,
            })
        }
        PrimitiveLayout::Function(f) => {
            // to resolve a function, type, we need to get the return type and argument types
            let return_type = resolve_entry_type_shallow(db, entry)?;
            let return_type = if f.return_type.is_none()
                && matches!(
                    return_type.layout.as_ref(),
                    Layout::Primitive(PrimitiveLayout::Unit(_))
                ) {
                // if the return type is Unit, we can just use None
                None
            } else {
                Some(return_type)
            };
            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type_shallow(db, c))
                .collect::<Result<Vec<_>>>()?;

            PrimitiveLayout::Function(FunctionLayout {
                return_type,
                arg_types,
            })
        }
        PrimitiveLayout::Pointer(pointer_def) => {
            let pointed_type = resolve_entry_type_shallow(db, entry)?;
            PrimitiveLayout::Pointer(PointerLayout {
                mutable: pointer_def.mutable,
                pointed_type,
            })
        }
        PrimitiveLayout::Reference(reference_def) => {
            let pointed_type = resolve_entry_type_shallow(db, entry)?;
            PrimitiveLayout::Reference(ReferenceLayout {
                mutable: reference_def.mutable,
                pointed_type,
            })
        }

        PrimitiveLayout::Tuple(_) => PrimitiveLayout::Tuple(resolve_tuple_type(db, entry)?),
    };

    Ok(layout)
}

/// Resolves a type entry to a `Def` _if_ the target entry is one of the
/// support "builtin" types -- these are types that we manually resolve rather
/// than relying on the DWARF info to do so.
fn resolve_as_builtin_type(db: &dyn DwarfDb, entry: Die) -> Result<Option<DieTypeDefinition>> {
    // when detecting builtin types, we look up the typename, which contains
    // our best effort at parsing the type definition based on the name
    // e.g. we detect `alloc::vec::Vec<u8>` as `VecDef` with
    // the primitive `u8` as the element type.

    // From there, we need to full resolve the type the DIE entry
    // (e.g. finding where the pointer + length for the Vec are stored)
    let Some(typename) = get_die_typename(db, TypeIndexEntry::new(db, entry)) else {
        tracing::debug!(
            "no name found for entry: {} at offset {}",
            entry.print(db),
            entry.offset()
        );

        return Ok(None);
    };

    tracing::debug!(
        "resolve_as_builtin_type: checking typename: {typename} {:#?} {}",
        typename.typedef,
        entry.location(db)
    );

    let layout = match &typename.typedef {
        Layout::Primitive(primitive_def) => {
            resolve_primitive_type(db, entry, primitive_def).map(|p| Some(Layout::Primitive(p)))
        }
        Layout::Std(std_def) => {
            // For std types, we need to do additional resolution based on DWARF
            // to get layout information, but we can use the parsed generic types
            match std_def {
                StdLayout::Option(_) => {
                    // Use the parsed Option type but resolve the actual layout
                    let option_def = resolve_option_type(db, entry)?;
                    Ok(Some(Layout::Std(StdLayout::Option(option_def))))
                }
                StdLayout::Vec(_) => {
                    // For Vec, we need to resolve the actual layout from DWARF
                    Ok(vec()
                        .parse(db, entry)
                        .map(|v| Some(Layout::Std(StdLayout::Vec(v))))?)
                }
                StdLayout::String(_) => {
                    // String has a known layout similar to Vec
                    resolve_string_type(db, entry).map(|s| Some(Layout::Std(StdLayout::String(s))))
                }
                StdLayout::Map(map) => {
                    // HashMap/BTreeMap need layout resolution
                    resolve_map_type(db, entry, map.variant.clone())
                        .map(|m| Some(Layout::Std(StdLayout::Map(m))))
                }
                StdLayout::Result(_) => {
                    // Result types need layout resolution
                    Ok(Some(Layout::Std(StdLayout::Result(resolve_result_type(
                        db, entry,
                    )?))))
                }
                StdLayout::SmartPtr(s) => {
                    // Smart pointers like Box, Rc, Arc need layout resolution
                    resolve_smart_ptr_type(db, entry, s.variant)
                        .map(|s| Some(Layout::Std(StdLayout::SmartPtr(s))))
                }
            }
        }
        Layout::Struct(_) => {
            // For custom structs, we don't handle them as builtins
            // They'll be resolved through the normal struct resolution path
            Ok(None)
        }
        Layout::Enum(_) => {
            // For custom enums, we don't handle them as builtins
            Ok(None)
        }
        Layout::CEnum(_) => {
            // C enums are handled as custom enums, not builtins
            Ok(None)
        }
        Layout::Alias { name: _ } => {
            // Aliases should be resolved through normal resolution
            Ok(None)
        }
    }?;

    Ok(layout.map(|l| TypeDefinition::new(entry, l)))
}

/// "Shallow" resolve a type -- if it's a primitive value, then
/// we'll return that directly. Otherwise, return an alias to some other
/// type entry (if we can find it).
// #[salsa::tracked]
pub fn shallow_resolve_type(db: &dyn DwarfDb, entry: Die) -> Result<DieTypeDefinition> {
    Ok(
        if let Some(builtin_ty) = resolve_as_builtin_type(db, entry)? {
            // we have a builtin type -- use it
            tracing::debug!("builtin: {builtin_ty:?}");
            builtin_ty
        } else {
            tracing::debug!("not a builtin type: {}", entry.print(db));
            TypeDefinition::new(
                entry,
                Layout::Alias {
                    name: entry.name(db).unwrap_or_else(|_| "unknown".to_string()),
                },
            )
        },
    )
}

fn resolve_enum_type(db: &dyn DwarfDb, entry: Die) -> Result<EnumLayout<Die>> {
    Ok(enum_def().parse(db, entry)?)
}

fn resolve_option_type(db: &dyn DwarfDb, entry: Die) -> Result<OptionLayout<Die>> {
    Ok(option_def().parse(db, entry)?)
}

fn resolve_result_type(db: &dyn DwarfDb, entry: Die) -> Result<ResultLayout<Die>> {
    Ok(result_def().parse(db, entry)?)
}

fn resolve_struct_type(db: &dyn DwarfDb, entry: Die) -> Result<StructLayout<Die>> {
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
            offset,
            ty,
        });
    }
    Ok(StructLayout {
        name,
        fields,
        size,
        alignment,
    })
}

/// Fully resolve a type from a DWARF DIE entry
#[salsa::tracked]

fn resolve_type_offset_tracked<'db>(
    db: &'db dyn DwarfDb,
    _file: DebugFile,
    entry: Die,
) -> Result<DieTypeDefinition> {
    if let Some(def) = resolve_as_builtin_type(db, entry)? {
        return Ok(def);
    }

    let layout = match entry.tag(db) {
        gimli::DW_TAG_base_type => {
            // unhandled primitive type
            tracing::debug!("unhandled primitive type: {}", entry.print(db));
            return Err(entry
                .format_with_location(db, "Primitive type not handled")
                .into());
        }
        gimli::DW_TAG_array_type => {
            let element_type = resolve_entry_type_shallow(db, entry)?;
            let children = entry.children(db)?;
            let subrange = children
                .iter()
                .find(|c| c.tag(db) == gimli::DW_TAG_subrange_type)
                .ok_or_else(|| {
                    entry.format_with_location(db, "array type missing subrange information")
                })?;
            let count = subrange.udata_attr(db, gimli::DW_AT_count)?;
            Layout::Primitive(PrimitiveLayout::Array(ArrayLayout {
                element_type,
                length: count,
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
                Layout::Enum(resolve_enum_type(db, entry)?)
            } else {
                // we have a struct type and it's _not_ a builtin -- we'll handle it now
                Layout::Struct(resolve_struct_type(db, entry)?)
            }
        }
        gimli::DW_TAG_subroutine_type => {
            let return_type = entry
                .get_referenced_entry(db, gimli::DW_AT_type)
                .ok()
                .map(|ty| resolve_entry_type_shallow(db, ty))
                .transpose()?;

            let arg_types = entry
                .children(db)?
                .into_iter()
                .filter(|c| c.tag(db) == gimli::DW_TAG_formal_parameter)
                .map(|c| resolve_entry_type_shallow(db, c))
                .collect::<Result<Vec<_>>>()?;

            Layout::Primitive(PrimitiveLayout::Function(FunctionLayout {
                return_type,
                arg_types,
            }))
        }
        gimli::DW_TAG_enumeration_type => Layout::CEnum(c_enum_def().parse(db, entry)?),
        t => {
            return Err(entry
                .format_with_location(db, format!("unsupported type: {t}"))
                .into());
        }
    };
    Ok(TypeDefinition::new(entry, layout))
}

pub fn resolve_type_offset(db: &dyn DwarfDb, entry: Die) -> Result<DieTypeDefinition> {
    resolve_type_offset_tracked(db, entry.file, entry)
}

#[cfg(test)]
mod test {
    use rudy_types::StdLayout;

    use crate::{function::resolve_function_variables, test_utils, types::DieLayout};

    #[test]
    fn test_std_type_detection() {
        test_utils::init_tracing();

        let _guard = test_utils::init_tracing_and_insta();

        let artifacts = test_utils::artifacts_dir(Some("aarch64-apple-darwin"));
        let db = test_utils::test_db(Some("aarch64-apple-darwin"));
        let db = &db;
        let binary = test_utils::load_binary(db, artifacts.join("examples/std_types"));

        let (_, symbol_index) = crate::symbols::index_symbol_map(db, binary).unwrap();

        tracing::debug!("Function index: {:#?}", symbol_index.functions);

        let (test_fn, symbol) = symbol_index
            .functions
            .get("test_fn")
            .expect("test_fn not found in symbols")
            .first_key_value()
            .expect("test_fn not found in symbols");

        let debug_file = symbol.debug_file;

        let fie = symbol_index
            .function_index(db, debug_file)
            .unwrap()
            .by_symbol_name(db)
            .get(test_fn)
            .expect("test_fn not found in function index");

        let params =
            resolve_function_variables(db, *fie).expect("Failed to resolve function variables");
        assert_eq!(params.params.len(), 3, "Expected 3 parameters in test_fn");

        let mut settings = insta::Settings::clone_current();
        settings.set_prepend_module_to_snapshot(false);
        test_utils::add_filters(&mut settings);
        // Check if we can resolve the types of the parameters
        let params = params.params;
        assert_eq!(params.len(), 3, "Expected 3 parameters in test_fn");

        let string_param = &params[0];
        let vec_param = &params[1];
        let map_param = &params[2];

        let string_type = string_param.ty.layout.as_ref();
        assert!(
            matches!(string_type, DieLayout::Std(StdLayout::<_>::String(_))),
            "Expected first parameter to be a String, got: {string_type:?}"
        );

        let vec_type = vec_param.ty.layout.as_ref();
        assert!(
            matches!(vec_type, DieLayout::Std(StdLayout::<_>::Vec(_))),
            "Expected second parameter to be a Vec, got: {vec_type:?}"
        );

        let map_type = map_param.ty.layout.as_ref();
        assert!(
            matches!(map_type, DieLayout::Std(StdLayout::<_>::Map(_))),
            "Expected third parameter to be a Map, got: {map_type:?}"
        );

        // Test resolving variables if we can find any
        // This is a basic smoke test to make sure the new get_die_name function works
        tracing::info!("DebugInfo appears to be working correctly");
    }
}
