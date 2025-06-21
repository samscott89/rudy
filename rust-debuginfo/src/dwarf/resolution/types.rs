//! Type resolution from DWARF debugging information

use std::sync::Arc;

use crate::database::Db;
use crate::dwarf::Die;
use crate::dwarf::index::get_die_typename;
use crate::typedef::{
    FloatDef, IntDef, OptionDef, PrimitiveDef, StdDef, StrSliceDef, StringDef, StructDef,
    StructField, TypeDef, TypeRef, UnitDef, UnsignedIntDef,
};

use anyhow::Context;

type Result<T> = std::result::Result<T, super::Error>;

/// Resolve the full type for a DIE entry
pub fn resolve_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    resolve_type_offset(db, type_entry)
}

/// Resolve the type for a DIE entry with shallow resolution
pub fn resolve_type_shallow<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    let type_entry = entry
        .get_unit_ref(db, gimli::DW_AT_type)?
        .with_context(|| entry.format_with_location(db, "Failed to get type entry"))?;
    shallow_resolve_type(db, type_entry)
}

/// Resolve Vec type layout from DWARF
fn resolve_vec_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    // Vec has a similar layout to &[T] - pointer + length + capacity
    let Some(size) = entry
        .get_attr(db, gimli::DW_AT_byte_size)
        .and_then(|attr| attr.udata_value())
    else {
        return Err(entry.format_with_location(db, "Vec size not found").into());
    };

    // For now, we'll use the parsed type information and just validate the layout
    // In a full implementation, we'd extract the exact field offsets
    let Some(typename) = get_die_typename(db, entry) else {
        return Err(entry
            .format_with_location(db, "Vec typename not found")
            .into());
    };

    match &typename.typedef {
        TypeDef::Std(StdDef::Vec(vec_def)) => Ok(TypeDef::Std(StdDef::Vec(vec_def.clone()))),
        _ => Err(entry
            .format_with_location(db, "Expected Vec type in typename")
            .into()),
    }
}

/// Resolve String type layout from DWARF
fn resolve_string_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    // String has the same layout as Vec<u8>
    let Some(_size) = entry
        .get_attr(db, gimli::DW_AT_byte_size)
        .and_then(|attr| attr.udata_value())
    else {
        return Err(entry
            .format_with_location(db, "String size not found")
            .into());
    };

    Ok(TypeDef::Std(StdDef::String(StringDef)))
}

/// Resolve Map type layout from DWARF
fn resolve_map_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
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

    match &typename.typedef {
        TypeDef::Std(StdDef::Map(map_def)) => {
            let mut resolved_map = map_def.clone();
            resolved_map.size = size as usize;
            Ok(TypeDef::Std(StdDef::Map(resolved_map)))
        }
        _ => Err(entry
            .format_with_location(db, "Expected Map type in typename")
            .into()),
    }
}

/// Resolve Result type layout from DWARF
fn resolve_result_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
    // Result is an enum type with Ok and Err variants
    let Some(typename) = get_die_typename(db, entry) else {
        return Err(entry
            .format_with_location(db, "Result typename not found")
            .into());
    };

    match &typename.typedef {
        TypeDef::Std(StdDef::Result(result_def)) => {
            Ok(TypeDef::Std(StdDef::Result(result_def.clone())))
        }
        _ => Err(entry
            .format_with_location(db, "Expected Result type in typename")
            .into()),
    }
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

fn resolve_str_type<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<TypeDef> {
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
        PrimitiveDef::Array(array_def) => todo!(),
        PrimitiveDef::Function(function_def) => todo!(),
        PrimitiveDef::Pointer(pointer_def) => todo!(),
        PrimitiveDef::Reference(reference_def) => todo!(),
        PrimitiveDef::Slice(slice_def) => todo!(),
        PrimitiveDef::Tuple(tuple_def) => todo!(),
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

    tracing::info!("resolve_as_builtin_type: checking typename: {}", typename);
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
                    resolve_vec_type(db, entry).map(Some)
                }
                StdDef::String(_) => {
                    // String has a known layout similar to Vec
                    resolve_string_type(db, entry).map(Some)
                }
                StdDef::Map(_) => {
                    // HashMap/BTreeMap need layout resolution
                    resolve_map_type(db, entry).map(Some)
                }
                StdDef::Result(_) => {
                    // Result types need layout resolution
                    resolve_result_type(db, entry).map(Some)
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
