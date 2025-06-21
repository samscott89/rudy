//! Data resolver trait for reading variables from memory during debugging.

use anyhow::{Context, Result};

use std::collections::BTreeMap;

use crate::database::Db;
use crate::typedef::{ArrayDef, DefKind, PointerDef, PrimitiveDef, StdDef, StrSliceDef, TypeDef};

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
    match &ty.kind {
        DefKind::Primitive(primitive_def) => {
            read_primitive_from_memory(db, address, &primitive_def, data_resolver)
        }
        DefKind::Struct(struct_def) => {
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
        DefKind::Std(std_def) => read_std_from_memory(db, address, &std_def, data_resolver),
        DefKind::Alias(entry) => {
            let die = entry.to_die(db);
            let def = crate::dwarf::resolve_type_offset(db, die)
                .with_context(|| format!("could not resolve type: {:?}", entry))?;

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

fn read_primitive_from_memory(
    db: &dyn Db,
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
            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;

            let mut address = address;
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
            read_from_memory(db, address, pointed_type, data_resolver)?
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

fn read_std_from_memory(
    db: &dyn Db,
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
