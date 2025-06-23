//! Data resolver trait for reading variables from memory during debugging.

use anyhow::{Context, Result};

use std::collections::BTreeMap;

use crate::database::Db;
use crate::typedef::{
    ArrayDef, PointerDef, PrimitiveDef, ReferenceDef, SliceDef, SmartPtrVariant, StdDef,
    StrSliceDef, TypeDef, VecDef,
};

/// Trait for resolving data from memory during debugging.
///
/// Implementors provide access to the target process's memory and registers,
/// allowing the debug info library to read variable values and follow pointers.
///
/// # Examples
///
/// ```no_run
/// use rust_debuginfo::DataResolver;
/// use anyhow::Result;
///
/// struct MyResolver {
///     base: u64,
///     // ... memory access implementation
/// }
///
/// impl DataResolver for MyResolver {
///     fn base_address(&self) -> u64 {
///         self.base
///     }
///     
///     fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
///         // Read from target process memory
///         todo!()
///     }
///     
///     fn get_registers(&self) -> Result<Vec<u64>> {
///         // Get current register values
///         todo!()
///     }
/// }
/// ```
pub trait DataResolver {
    /// Returns the base address for memory calculations.
    ///
    /// This is typically the base address where the binary is loaded in memory.
    /// All addresses returned by this trait should be adjusted by this base.
    fn base_address(&self) -> u64;

    /// Reads raw bytes from memory at the given address.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to read from
    /// * `size` - Number of bytes to read
    ///
    /// # Returns
    ///
    /// The bytes read from memory
    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>>;

    /// Reads a 64-bit address from memory.
    ///
    /// This method handles pointer dereferencing and base address adjustment.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to read the pointer from
    ///
    /// # Returns
    ///
    /// The dereferenced address, adjusted for the base address
    fn read_address(&self, address: u64) -> Result<u64> {
        let data = self.read_memory(address, std::mem::size_of::<u64>())?;
        if data.len() != std::mem::size_of::<u64>() {
            return Err(anyhow::anyhow!("Failed to read address"));
        }
        let addr = u64::from_le_bytes(data.try_into().unwrap());
        tracing::trace!("read raw address: {addr:#x}");
        if addr == 0 {
            Ok(0)
        } else {
            addr.checked_sub(self.base_address())
                .ok_or_else(|| anyhow::anyhow!("Address underflow when adjusting for base address"))
        }
    }
    /// Gets all register values from the target.
    ///
    /// The order and meaning of registers is architecture-specific.
    ///
    /// # Returns
    ///
    /// A vector of register values
    fn get_registers(&self) -> Result<Vec<u64>>;

    /// Gets a specific register value by index.
    ///
    /// # Arguments
    ///
    /// * `idx` - The register index (architecture-specific)
    ///
    /// # Returns
    ///
    /// The register value, adjusted for the base address
    fn get_register(&self, idx: usize) -> Result<u64> {
        let registers = self.get_registers()?;
        registers
            .get(idx)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("Invalid register index: {idx} (max: {})", registers.len())
            })
            .and_then(|addr| {
                // Adjust the address based on the base address
                addr.checked_sub(self.base_address()).ok_or_else(|| {
                    anyhow::anyhow!("Address underflow when adjusting for base address")
                })
            })
    }
}

pub fn read_from_memory<'db>(
    db: &'db dyn Db,
    address: u64,
    ty: &TypeDef,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    tracing::debug!("read_from_memory {address:#x} {}", ty.display_name());
    match &ty {
        TypeDef::Primitive(primitive_def) => {
            read_primitive_from_memory(db, address, &primitive_def, data_resolver)
        }
        TypeDef::Struct(struct_def) => {
            let mut fields = BTreeMap::new();
            for field in &struct_def.fields {
                let field_name = &field.name;
                let field_ty = &field.ty;
                let field_address = address + (field.offset as u64);
                let field_value = read_from_memory(db, field_address, field_ty, data_resolver)?;
                fields.insert(field_name.to_string(), field_value);
            }
            Ok(crate::Value::Struct {
                ty: struct_def.name.clone(),
                fields,
            })
        }
        TypeDef::Std(std_def) => read_std_from_memory(db, address, &std_def, data_resolver),
        TypeDef::Alias(entry) => {
            let die = entry.to_die(db);
            let def = crate::dwarf::resolve_type_offset(db, die)
                .with_context(|| format!("could not resolve type: {:?}", entry))?;

            read_from_memory(db, address, &def, data_resolver)
        }
        TypeDef::Enum(enum_def) => {
            todo!("read_from_memory: EnumDef not implemented yet: {enum_def:#?}")
        }
        TypeDef::Other { name } => {
            tracing::warn!("read_from_memory: unsupported type {name}");
            Err(anyhow::anyhow!("Unsupported type: {name}"))
        }
    }
}

fn read_primitive_from_memory(
    db: &dyn Db,
    address: u64,
    def: &PrimitiveDef,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    let value = match def {
        PrimitiveDef::Bool(_) => {
            let memory = data_resolver.read_memory(address, 1)?;
            let bool_value = memory[0] != 0;
            crate::Value::Scalar {
                ty: "bool".to_string(),
                value: bool_value.to_string(),
            }
        }
        PrimitiveDef::Char(()) => {
            let memory = data_resolver.read_memory(address, 4)?;
            let char_value = char::from_u32(u32::from_le_bytes(memory.try_into().unwrap()))
                .ok_or_else(|| anyhow::anyhow!("Invalid char value at address {address:#x}"))?;
            crate::Value::Scalar {
                ty: "char".to_string(),
                value: format!("'{char_value}'"),
            }
        }
        PrimitiveDef::Function(function_def) => crate::Value::Scalar {
            ty: function_def.display_name(),
            value: format!("fn at {address:#x}"),
        },
        PrimitiveDef::Array(ArrayDef {
            element_type,
            length,
        }) => {
            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;

            let mut address = address;
            let mut values = Vec::with_capacity(*length);
            for _ in 0..*length {
                let value = read_from_memory(db, address, element_type, data_resolver)?;
                values.push(value);
                address += element_size;
            }
            crate::Value::Array {
                ty: format!("[{}; {length}]", element_type.display_name()),
                items: values,
            }
        }
        PrimitiveDef::Pointer(PointerDef { pointed_type, .. }) => {
            let address = data_resolver.read_address(address)?;
            read_from_memory(db, address, pointed_type, data_resolver)?.prefix_type("*")
        }
        PrimitiveDef::Reference(ReferenceDef { pointed_type, .. }) => {
            let address = data_resolver.read_address(address)?;
            read_from_memory(db, address, pointed_type, data_resolver)?.prefix_type("&")
        }
        PrimitiveDef::Slice(SliceDef {
            element_type,
            data_ptr_offset,
            length_offset,
            size: _,
        }) => {
            let length = address + *length_offset as u64;
            let length_bytes = data_resolver.read_memory(length, 8)?;
            let length = u64::from_le_bytes(length_bytes.try_into().unwrap());
            tracing::debug!("length: {length}");

            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;
            let mut data_ptr = address + *data_ptr_offset as u64;
            let mut values = Vec::with_capacity(length as usize);
            for _ in 0..length {
                let value = read_from_memory(db, data_ptr, element_type, data_resolver)?;
                values.push(value);
                data_ptr += element_size;
            }
            crate::Value::Array {
                ty: format!("&[{}]", element_type.display_name()),
                items: values,
            }
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
                16 => u128::from_le_bytes(memory.try_into().unwrap()).to_string(),
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported UnsignedIntDef size {} at address {address:#x}",
                        unsigned_int_def.size
                    )
                }
            };
            crate::Value::Scalar {
                ty: unsigned_int_def.display_name(),
                value: num_string,
            }
        }
        PrimitiveDef::Float(float_def) => {
            let memory = data_resolver.read_memory(address, float_def.size)?;

            let num_string = match float_def.size {
                4 => f32::from_le_bytes(memory.try_into().unwrap()).to_string(),
                8 => f64::from_le_bytes(memory.try_into().unwrap()).to_string(),
                16 => {
                    anyhow::bail!("f128 is not supported yet, found at address {address:#x}");
                }
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported FloatDef size {} at address {address:#x}",
                        float_def.size
                    )
                }
            };
            crate::Value::Scalar {
                ty: format!("f{}", float_def.size * 8),
                value: num_string,
            }
        }
        PrimitiveDef::Int(int_def) => {
            let memory = data_resolver.read_memory(address, int_def.size)?;

            let num_string = match int_def.size {
                1 => i8::from_le_bytes(memory.try_into().unwrap()).to_string(),
                2 => i16::from_le_bytes(memory.try_into().unwrap()).to_string(),
                4 => i32::from_le_bytes(memory.try_into().unwrap()).to_string(),
                8 => i64::from_le_bytes(memory.try_into().unwrap()).to_string(),
                16 => i128::from_le_bytes(memory.try_into().unwrap()).to_string(),
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported IntDef size {} at address {address:#x}",
                        int_def.size
                    )
                }
            };
            crate::Value::Scalar {
                ty: int_def.display_name(),
                value: num_string,
            }
        }
        PrimitiveDef::Never(_) => {
            // The Never type is a zero-sized type, so we return a placeholder value.
            crate::Value::Scalar {
                ty: "Never".to_string(),
                value: "unreachable".to_string(),
            }
        }
        PrimitiveDef::Str(()) => {
            todo!("read_primitive_from_memory: bare `str` is not supported yet");
        }
        PrimitiveDef::Tuple(tuple_def) => {
            todo!("read_primitive_from_memory: TupleDef not implemented yet: {tuple_def:#?}");
        }
        PrimitiveDef::Unit(_) => {
            // The Unit type is a zero-sized type, so we return a placeholder value.
            crate::Value::Scalar {
                ty: "()".to_string(),
                value: "()".to_string(),
            }
        }
    };

    Ok(value)
}

fn read_std_from_memory(
    db: &dyn Db,
    address: u64,
    def: &StdDef,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Value> {
    let value = match def {
        StdDef::Option(option_def) => {
            let first_byte = data_resolver.read_memory(address, 1)?;
            if first_byte[0] == 0 {
                return Ok(crate::Value::Scalar {
                    ty: "Option".to_string(),
                    value: "None".to_string(),
                });
            }

            read_from_memory(
                db,
                address + option_def.some_offset as u64,
                &option_def.inner_type,
                data_resolver,
            )?
            .wrap_type("Option::Some")
        }
        StdDef::Vec(VecDef {
            length_offset,
            data_ptr_offset,
            inner_type,
        }) => {
            tracing::debug!(
                "reading Vec at {address:#x}, length_offset: {length_offset:#x}, data_ptr_offset: {data_ptr_offset:#x}",
            );
            let element_type = inner_type;
            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;

            let length = data_resolver
                .read_memory(address + *length_offset as u64, 8)?
                .try_into()
                .map(usize::from_le_bytes)
                .map_err(|_| {
                    anyhow::anyhow!("Failed to read length for Vec at address {address:#x}")
                })?;
            tracing::debug!("Vec length: {length}");
            let mut values = Vec::with_capacity(length);
            let mut address = data_resolver
                .read_address(address + *data_ptr_offset as u64)
                .with_context(|| {
                    format!(
                        "Failed to read Vec data pointer at {:#x}",
                        address + *data_ptr_offset as u64
                    )
                })?;
            tracing::debug!("reading Vec data at {address:#016x}");
            for _ in 0..length {
                let value = read_from_memory(db, address, element_type, data_resolver)?;
                values.push(value);
                address += element_size;
            }
            crate::Value::Array {
                ty: format!("Vec<{}>", element_type.display_name()),
                items: values,
            }
        }
        StdDef::String(s) => {
            let v = &s.0;
            tracing::debug!(
                "reading String length at {:#x}",
                address + v.length_offset as u64
            );
            let length = data_resolver
                .read_memory(address + v.length_offset as u64, 8)?
                .try_into()
                .map(usize::from_le_bytes)
                .map_err(|_| {
                    anyhow::anyhow!("Failed to read length for Vec at address {address:#x}")
                })?;
            tracing::debug!("String length: {length}");
            // read the bytes from the data pointer
            let data_address = address + s.0.data_ptr_offset as u64;
            let data = data_resolver.read_address(data_address).with_context(|| {
                format!("Failed to read String data pointer at {data_address:#x}")
            })?;
            tracing::debug!("reading String data at {data:#016x}");
            let bytes = data_resolver.read_memory(data, length)?;
            let value = String::from_utf8_lossy(&bytes).to_string();
            crate::Value::Scalar {
                ty: "String".to_string(),
                value: format!("\"{value}\""),
            }
        }
        StdDef::Map(def) => {
            todo!("read_std_from_memory: MapDef not implemented yet: {def:#?}")
        }
        StdDef::SmartPtr(s) => match s.variant {
            SmartPtrVariant::Box => {
                let inner_type = s.inner_type.clone();
                let address = data_resolver.read_address(address)?;
                read_from_memory(db, address, &inner_type, data_resolver)?
                    .wrap_type(s.variant.name())
            }
            SmartPtrVariant::Rc | SmartPtrVariant::Arc => {
                let inner_type = s.inner_type.clone();
                let inner_address =
                    data_resolver.read_address(address + s.inner_ptr_offset as u64)?;
                let data_address = inner_address + s.data_ptr_offset as u64;
                read_from_memory(db, data_address, &inner_type, data_resolver)?
                    .wrap_type(s.variant.name())
            }
            _ => {
                todo!("read_std_from_memory: SmartPtrVariant not implemented yet: {s:#?}")
            }
        },
        StdDef::Result(result_def) => {
            todo!("read_std_from_memory: ResultDef not implemented yet: {result_def:#?}")
        }
    };

    Ok(value)
}
