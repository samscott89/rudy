//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::data;
use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::index::get_die_typename;
use crate::typedef::{
    ArrayDef, MapDef, OptionDef, PointerDef, PrimitiveDef, ReferenceDef, ResultDef, StdDef,
    StrSliceDef, StringDef, StructDef, StructField, TypeDef, TypeRef, VecDef,
};

use anyhow::Context;

type Result<T> = std::result::Result<T, super::Error>;

/// Resolve the full type for a DIE entry
pub fn resolve_entry_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_entry_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve Vec type layout from DWARF
fn resolve_vec_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<VecDef> {
    let mut data_ptr_offset = 0;
    // Vec.buf -> RawVec
    let buf = entry.get_member(db, "buf").context("could not find buf")?;
    data_ptr_offset += buf
        .udata_attr(db, gimli::DW_AT_data_member_location)
        .context("could not find buf offset")?;

    let rawvec = buf
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| buf.format_with_location(db, "Failed to get type for buf"))?;

    // RawVec.inner -> RawVecInner

    let inner = rawvec
        .get_member(db, "inner")
        .with_context(|| buf.format_with_location(db, "Failed to get inner member"))?;
    data_ptr_offset += inner
        .udata_attr(db, gimli::DW_AT_data_member_location)
        .context("could not find inner offset")?;

    let rawvec_inner = inner
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| inner.format_with_location(db, "Failed to get type for inner"))?;

    // RawVecInner.ptr -> *mut T
    let data_ptr = rawvec_inner
        .get_member_attribute(db, "ptr", gimli::DW_AT_data_member_location)
        .context("could not find ptr")?
        .udata_value()
        .map(|v| v as usize)
        .context("ptr offset is not a valid udata")?;

    data_ptr_offset += data_ptr;

    let length_offset = entry
        .get_member_attribute(db, "len", gimli::DW_AT_data_member_location)
        .context("could not find len")?;
    let length_offset = length_offset
        .udata_value()
        .map(|v| v as usize)
        .context("len offset is not a valid udata")?;

    let inner_type = entry
        .get_generic_type_entry(db, "T")
        .context("could not find inner type")?;
    tracing::debug!(
        "inner type for Vec: {}",
        inner_type.format_with_location(db, inner_type.print(db))
    );
    let inner_type =
        shallow_resolve_type(db, inner_type).context("could not resolve inner type of vec")?;

    Ok(VecDef {
        data_ptr_offset,
        length_offset,
        inner_type: Arc::new(inner_type),
    })
}

/// Resolve String type layout from DWARF
fn resolve_string_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<StringDef> {
    // string should just have a single child member for the vec
    let vec_field = entry
        .get_member(db, "vec")
        .context("could not find vec field for string")?;
    let vec_type = vec_field
        .get_unit_ref(db, gimli::DW_AT_type)
        .context("could not get type for vec field")?
        .with_context(|| vec_field.format_with_location(db, "Failed to get type for vec field"))?;
    let vec = resolve_vec_type(db, vec_type)
        .with_context(|| vec_type.format_with_location(db, "Failed to resolve vec for string"))?;

    Ok(StringDef(vec))
}

/// Resolve Map type layout from DWARF
fn resolve_map_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<MapDef> {
    let Some(size) = entry
        .get_attr(db, gimli::DW_AT_byte_size)
        .and_then(|attr| attr.udata_value())
    else {
        return Err(entry.format_with_location(db, "Map size not found").into());
    };

    let Some(typename) = get_die_typename(db, entry) else {
        return Err(entry
            .format_with_location(db, "Map typename not found")
            .into());
    };

    // match &typename.typedef {
    //     TypeDef::Std(StdDef::Map(map_def)) => {
    //         let mut resolved_map = map_def.clone();
    //         resolved_map.size = size as usize;
    //         Ok(TypeDef::Std(StdDef::Map(resolved_map)))
    //     }
    //     _ => Err(entry
    //         .format_with_location(db, "Expected Map type in typename")
    //         .into()),
    // }
    todo!()
}

/// Resolve Result type layout from DWARF
fn resolve_result_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<ResultDef> {
    // Result is an enum type with Ok and Err variants
    let Some(typename) = get_die_typename(db, entry) else {
        return Err(entry
            .format_with_location(db, "Result typename not found")
            .into());
    };

    // match &typename.typedef {
    //     TypeDef::Std(StdDef::Result(result_def)) => {
    //         Ok(TypeDef::Std(StdDef::Result(result_def.clone())))
    //     }
    //     _ => Err(entry
    //         .format_with_location(db, "Expected Result type in typename")
    //         .into()),
    // }
    todo!()
}

/// Resolve smart pointer type layout from DWARF
fn resolve_smart_ptr_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let Some(typename) = get_die_typename(db, entry) else {
        return Err(entry
            .format_with_location(db, "SmartPtr typename not found")
            .into());
    };

    match &typename.typedef {
        TypeDef::Std(StdDef::SmartPtr(smart_ptr_def)) => {
            Ok(TypeDef::Std(StdDef::SmartPtr(smart_ptr_def.clone())))
        }
        _ => Err(entry
            .format_with_location(db, "Expected SmartPtr type in typename")
            .into()),
    }
}

/// Resolve Option type structure
fn resolve_option_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<OptionDef> {
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
                    // the struct type for the `Some` variant
                    let member_type = child
                        .get_generic_type_entry(db, "T")
                        .context("could not find generic type T for Some")?;
                    let inner_type = resolve_type_offset(db, member_type)
                        .context("could not resolve inner type for Some")?;

                    let some_offset = child
                        .get_member_attribute(db, "__0", gimli::DW_AT_data_member_location)
                        .context("could not find __0 offset for Some")?
                        .udata_value()
                        .map(|v| v as u64)
                        .context("__0 offset is not a valid udata")?;

                    return Ok(OptionDef {
                        inner_type: Arc::new(inner_type),
                        discriminant_offset: 0, // discriminant is always at offset 0
                        some_offset: some_offset as usize,
                    });
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

fn resolve_str_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let Some(size) = entry
        .get_attr(db, gimli::DW_AT_byte_size)
        .and_then(|attr| attr.udata_value())
    else {
        return Err(entry
            .format_with_location(db, "struct size not found")
            .into());
    };
    let data_ptr_offset = entry
        .get_member_attribute(db, "data_ptr", gimli::DW_AT_data_member_location)
        .context("could not find data_ptr")?;
    let data_ptr_offset = data_ptr_offset
        .udata_value()
        .map(|v| v as usize)
        .context("data_ptr offset is not a valid udata")?;

    let length_offset = entry
        .get_member_attribute(db, "length", gimli::DW_AT_data_member_location)
        .context("could not find length")?;
    let length_offset = length_offset
        .udata_value()
        .map(|v| v as usize)
        .context("length offset is not a valid udata")?;

    Ok(TypeDef::Primitive(PrimitiveDef::StrSlice(StrSliceDef {
        data_ptr_offset,
        length_offset,
        size: size as usize,
    })))
}

fn resolve_as_primitive_type<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    primitive_def: &PrimitiveDef,
) -> Result<Option<TypeDef>> {
    match primitive_def {
        // these "scalar" types are already fully resolved
        PrimitiveDef::Int(_)
        | PrimitiveDef::Bool(_)
        | PrimitiveDef::Char(_)
        | PrimitiveDef::Float(_)
        | PrimitiveDef::Never(_)
        | PrimitiveDef::Str(_)
        | PrimitiveDef::UnsignedInt(_)
        | PrimitiveDef::Unit(_) => Ok(Some(TypeDef::Primitive(primitive_def.clone()))),

        // these types need to be resolved further
        PrimitiveDef::StrSlice(_) => resolve_str_type(db, entry).map(Some),
        PrimitiveDef::Array(_array_def) => todo!(
            "array def at:\n\n{}",
            entry.format_with_location(db, entry.print(db))
        ),
        PrimitiveDef::Function(_function_def) => {
            todo!(
                "function def at:\n\n{}",
                entry.format_with_location(db, entry.print(db))
            )
        }
        PrimitiveDef::Pointer(pointer_def) => {
            let inner = resolve_entry_type(db, entry)?;
            Ok(Some(TypeDef::Primitive(PrimitiveDef::Pointer(
                PointerDef {
                    mutable: pointer_def.mutable,
                    pointed_type: Arc::new(inner),
                },
            ))))
        }
        PrimitiveDef::Reference(reference_def) => {
            let inner = resolve_entry_type(db, entry)?;
            Ok(Some(TypeDef::Primitive(PrimitiveDef::Reference(
                ReferenceDef {
                    mutable: reference_def.mutable,
                    pointed_type: Arc::new(inner),
                },
            ))))
        }
        PrimitiveDef::Slice(_slice_def) => {
            todo!(
                "slice def at:\n\n{}",
                entry.format_with_location(db, entry.print(db))
            )
        }
        PrimitiveDef::Tuple(_tuple_def) => {
            todo!(
                "tuple def at:\n\n{}",
                entry.format_with_location(db, entry.print(db))
            )
        }
    }
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

    tracing::info!("resolve_as_builtin_type: checking typename: {typename}",);
    tracing::info!("resolve_as_builtin_type: typedef: {:?}", typename.typedef);

    match &typename.typedef {
        TypeDef::Primitive(primitive_def) => resolve_as_primitive_type(db, entry, primitive_def),
        TypeDef::Std(std_def) => {
            // For std types, we need to do additional resolution based on DWARF
            // to get layout information, but we can use the parsed generic types
            match std_def {
                StdDef::Option(_) => {
                    // Use the parsed Option type but resolve the actual layout
                    let option_def = resolve_option_type(db, entry)?;
                    Ok(Some(TypeDef::Std(StdDef::Option(option_def))))
                }
                StdDef::Vec(_) => {
                    // For Vec, we need to resolve the actual layout from DWARF
                    resolve_vec_type(db, entry).map(|v| Some(TypeDef::Std(StdDef::Vec(v))))
                }
                StdDef::String(_) => {
                    // String has a known layout similar to Vec
                    resolve_string_type(db, entry).map(|s| Some(TypeDef::Std(StdDef::String(s))))
                }
                StdDef::Map(_) => {
                    // HashMap/BTreeMap need layout resolution
                    resolve_map_type(db, entry).map(|m| Some(TypeDef::Std(StdDef::Map(m))))
                }
                StdDef::Result(_) => {
                    // Result types need layout resolution
                    resolve_result_type(db, entry).map(|r| Some(TypeDef::Std(StdDef::Result(r))))
                }
                StdDef::SmartPtr(_) => {
                    // Smart pointers like Box, Rc, Arc need layout resolution
                    resolve_smart_ptr_type(db, entry).map(Some)
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
pub fn shallow_resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    Ok(
        if let Some(builtin_ty) = resolve_as_builtin_type(db, entry)? {
            // we have a builtin type -- use it
            tracing::debug!("builtin: {builtin_ty:?}");
            builtin_ty
        } else {
            TypeDef::Alias(TypeRef::from_die(&entry, db))
        },
    )
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
            let children = entry.children(db);
            let subrange = children
                .iter()
                .find(|c| c.tag(db) == gimli::DW_TAG_subrange_type)
                .ok_or_else(|| {
                    entry.format_with_location(db, "array type missing subrange information")
                })?;
            let Some(count) = subrange
                .get_attr(db, gimli::DW_AT_count)
                .and_then(|attr| attr.udata_value())
            else {
                return Err(subrange
                    .format_with_location(db, "array type subrange count not found")
                    .into());
            };
            TypeDef::Primitive(PrimitiveDef::Array(ArrayDef {
                element_type: Arc::new(array_type),
                length: count as usize,
            }))
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
                        tracing::debug!("skipping non-member entry: {}", child.print(db));
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
                TypeDef::Struct(StructDef {
                    name,
                    fields,
                    size: size as usize,
                    alignment: alignment as usize,
                })
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
