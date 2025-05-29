use std::collections::BTreeMap;

use anyhow::{Context, Result};

use crate::{
    data::{DefKind, PrimitiveDef, TypeDef},
    database::Db,
};

use super::{ArrayDef, PointerDef, StdDef, StrSliceDef};

pub fn read_from_memory<'db>(
    db: &'db dyn Db,
    address: u64,
    ty: &TypeDef<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    tracing::debug!("read_from_memory {address:#x} {}", ty.display_name(db));
    match ty.kind(db) {
        DefKind::Primitive(primitive_def) => {
            read_primitive_from_memory(db, address, primitive_def, data_resolver)
        }
        DefKind::Struct(struct_def) => {
            let mut fields = BTreeMap::new();
            for field in &struct_def.fields {
                let field_name = &field.name;
                let field_ty = &field.ty;
                let field_address = address + (field.offset as u64);
                let field_value = read_from_memory(db, field_address, &field_ty, data_resolver)?;
                fields.insert(field_name.to_string(), field_value);
            }
            Ok(crate::Value::Struct {
                ty: struct_def.name.clone(),
                fields,
            })
        }
        DefKind::Std(std_def) => read_std_from_memory(db, address, std_def, data_resolver),
        DefKind::Alias(entry) => {
            let def = crate::dwarf::resolve_type_offset(db, *entry)
                .with_context(|| format!("could not resolve type: {}", entry.print(db)))?;

            read_from_memory(db, address, &def, data_resolver)
        }
        d => {
            todo!("read_from_memory: {d:#?}");
        } // TypeData::String => todo!(),
          // TypeData::Struct { .. } => todo!(),
          // TypeData::Unsupported => todo!(),
          // TypeData::Pointer(_) => todo!(),
    }
}

fn read_primitive_from_memory<'db>(
    db: &'db dyn Db,
    address: u64,
    def: &PrimitiveDef,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    let value = match def {
        // PrimitiveDef::Array(array_def) => todo!(),
        // PrimitiveDef::Bool(bool_def) => todo!(),
        // PrimitiveDef::Char(char_def) => todo!(),
        // PrimitiveDef::Float(float_def) => todo!(),
        // PrimitiveDef::Function(function_def) => todo!(),
        // PrimitiveDef::Int(int_def) => todo!(),
        // PrimitiveDef::Reference(reference_def) => todo!(),
        // PrimitiveDef::Slice(slice_def) => todo!(),
        // PrimitiveDef::Str(str_def) => todo!(),
        // PrimitiveDef::Tuple(tuple_def) => todo!(),
        // PrimitiveDef::Unit(unit_def) => todo!(),
        PrimitiveDef::Array(ArrayDef {
            element_type,
            length,
        }) => {
            let mut values = Vec::new();
            let element_size = element_type.size(db)?.with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name(db)
                )
            })? as u64;

            let mut address = address;
            for _ in 0..*length {
                let value = read_from_memory(db, address, &element_type, data_resolver)?;
                values.push(value);
                address += element_size;
            }
            crate::Value::Array {
                ty: format!("[{}; {length}]", element_type.display_name(db)),
                items: values,
            }
        }
        PrimitiveDef::Pointer(PointerDef { pointed_type }) => {
            let address = data_resolver.read_address(address)?;
            read_from_memory(db, address, &pointed_type, data_resolver)?
        }
        PrimitiveDef::StrSlice(StrSliceDef {
            data_ptr_offset,
            length_offset,
            size: _,
        }) => {
            let data_ptr = address + *data_ptr_offset as u64;
            let length = address + *length_offset as u64;

            let data_address = data_resolver.read_address(data_ptr)?;
            let memory = data_resolver.read_memory(length, 8)?;
            let length = u64::from_le_bytes(memory.try_into().unwrap());
            tracing::debug!("length: {length}");

            let memory = data_resolver.read_memory(data_address, length as usize)?;
            let string_value = String::from_utf8_lossy(&memory).to_string();
            crate::Value::Scalar {
                ty: "str".to_string(),
                value: format!("\"{string_value}\""),
            }
        }
        PrimitiveDef::UnsignedInt(unsigned_int_def) => {
            let memory = data_resolver.read_memory(address, unsigned_int_def.size)?;

            let num_string = match unsigned_int_def.size {
                1 => u8::from_le_bytes(memory.try_into().unwrap()).to_string(),
                2 => u16::from_le_bytes(memory.try_into().unwrap()).to_string(),
                4 => u32::from_le_bytes(memory.try_into().unwrap()).to_string(),
                8 => u64::from_le_bytes(memory.try_into().unwrap()).to_string(),
                _ => todo!("read_primitive_from_memory: {unsigned_int_def:#?}"),
            };
            crate::Value::Scalar {
                ty: format!("u{}", unsigned_int_def.size * 8),
                value: num_string,
            }
        }
        def => {
            todo!("read_primitive_from_memory: {def:#?}");
        }
    };

    Ok(value)
}

fn read_std_from_memory<'db>(
    db: &'db dyn Db,
    address: u64,
    def: &StdDef,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    let value = match def {
        StdDef::Option(option_def) => {
            let address = data_resolver.read_address(address)?;
            if address == 0 {
                return Ok(crate::Value::Scalar {
                    ty: "Option".to_string(),
                    value: "None".to_string(),
                });
            }

            read_from_memory(db, address, &option_def.inner_type, data_resolver)?
        }
        def => {
            todo!("read_std_from_memory: {def:#?}");
        }
    };

    Ok(value)
}
