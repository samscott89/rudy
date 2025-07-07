//! Data resolver trait for reading variables from memory during debugging.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rudy_dwarf::{Die, types::DieTypeDefinition};
use rudy_types::{
    ArrayLayout, BTreeNodeLayout, CEnumLayout, EnumLayout, Layout, MapLayout, MapVariant,
    OptionLayout, PointerLayout, PrimitiveLayout, ReferenceLayout, SliceLayout, SmartPtrVariant,
    StdLayout, StrSliceLayout, VecLayout,
};

use crate::{Value, database::Db, outputs::TypedPointer};

/// Trait for resolving data from memory during debugging.
///
/// Implementors provide access to the target process's memory and registers,
/// allowing the debug info library to read variable values and follow pointers.
///
/// NOTE: All of the addresses passed to/return from this trait will be relative
/// to the target _binary_ and does not account for ASLR (Address Space Layout Randomization).
pub trait DataResolver {
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
    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        Err(anyhow::anyhow!(
            "read_memory({address:#x}, {size}) not implemented for this DataResolver",
        ))
    }

    /// Reads a 64-bit address from memory.
    ///
    /// This method handles pointer dereferencing.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to read the pointer from
    ///
    /// # Returns
    ///
    /// The dereferenced address
    fn read_address(&self, address: u64) -> Result<u64> {
        let data = self.read_memory(address, std::mem::size_of::<u64>())?;
        if data.len() != std::mem::size_of::<u64>() {
            return Err(anyhow::anyhow!("Failed to read address"));
        }
        let addr = u64::from_le_bytes(data.try_into().unwrap());
        tracing::trace!("read raw address: {addr:#x}");
        Ok(addr)
    }
    /// Gets a specific register value by index.
    ///
    /// # Arguments
    ///
    /// * `idx` - The register index (architecture-specific)
    ///
    /// # Returns
    ///
    /// The register value
    fn get_register(&self, idx: usize) -> Result<u64> {
        Err(anyhow::anyhow!(
            "get_register({idx}) not implemented for this DataResolver"
        ))
    }

    fn get_stack_pointer(&self) -> Result<u64> {
        Err(anyhow::anyhow!("get_stack_pointer() not implemented"))
    }

    /// Allocates memory in the target process.
    ///
    /// # Arguments
    ///
    /// * `size` - Number of bytes to allocate
    ///
    /// # Returns
    ///
    /// The address of the allocated memory
    fn allocate_memory(&self, size: usize) -> Result<u64> {
        Err(anyhow::anyhow!(
            "allocate_memory({size:#x}) not implemented"
        ))
    }

    /// Writes data to memory in the target process.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to write to
    /// * `data` - The bytes to write
    fn write_memory(&self, address: u64, data: &[u8]) -> Result<()> {
        let _ = data;
        Err(anyhow::anyhow!(
            "write_memory({address:#x}, &[..]) not implemented"
        ))
    }
}

pub(crate) struct DataResolverExpressionContext<'a, T: ?Sized>(pub &'a T);

impl<'a, R: DataResolver + ?Sized> rudy_dwarf::expressions::ExpressionContext
    for DataResolverExpressionContext<'a, R>
{
    fn get_register(&self, register: u16) -> Result<u64> {
        self.0.get_register(register as usize)
    }

    fn get_stack_pointer(&self) -> Result<u64> {
        self.0.get_stack_pointer()
    }
}

/// Returns a list of map entries from a memory address.
pub fn read_map_entries(
    address: u64,
    def: &MapLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Vec<(TypedPointer, TypedPointer)>> {
    tracing::trace!("read_map_entries {address:#x} {}", def.display_name());

    match def.variant.clone() {
        MapVariant::HashMap {
            bucket_mask_offset,
            ctrl_offset,
            items_offset,
            pair_size,
            key_offset,
            value_offset,
        } => {
            let bucket_mask_address = address + bucket_mask_offset as u64;
            let ctrl_address = address + ctrl_offset as u64;
            let items_address = address + items_offset as u64;
            // Read item count
            let items = data_resolver.read_memory(items_address, 8)?;
            let items = usize::from_le_bytes(items.try_into().unwrap());

            if items == 0 {
                return Ok(vec![]);
            }

            tracing::trace!(
                "reading HashMap at {address:#x}, items: {items}, bucket_mask_addr: {bucket_mask_address:#x}, ctrl_addr: {ctrl_address:#x}"
            );

            // Read bucket mask to get capacity
            let bucket_mask = data_resolver.read_memory(bucket_mask_address, 8)?;
            let capacity = usize::from_le_bytes(bucket_mask.try_into().unwrap()) + 1;

            // Read control bytes pointer
            let ctrl_ptr = data_resolver.read_address(ctrl_address)?;

            tracing::trace!(
                "HashMap capacity: {capacity}, ctrl_ptr: {ctrl_ptr:#x}, items: {items}"
            );

            // Calculate size of key-value pair
            // Data starts BEFORE the control bytes, counting backwards!
            let mut slot_addr = ctrl_ptr - pair_size as u64;

            // Read control bytes
            let ctrl_bytes = data_resolver.read_memory(ctrl_ptr, capacity)?;

            let mut entries = Vec::new();

            for &ctrl in &ctrl_bytes {
                if ctrl < 0x80 {
                    // Occupied slot
                    let key = TypedPointer {
                        address: slot_addr + key_offset as u64,
                        type_def: def.key_type.clone(),
                    };
                    let value = TypedPointer {
                        address: slot_addr + value_offset as u64,
                        type_def: def.value_type.clone(),
                    };

                    entries.push((key, value));

                    // Stop if we've found all items
                    if entries.len() >= items {
                        break;
                    }
                }
                // decrement address for the next slot
                slot_addr -= pair_size as u64;
            }
            Ok(entries)
        }
        MapVariant::BTreeMap {
            length_offset,
            root_offset,
            root_layout,
            node_layout,
        } => {
            // Read the length field
            let length_addr = address + length_offset as u64;
            let length_bytes = data_resolver.read_memory(length_addr, 8)?;
            let length = usize::from_le_bytes(length_bytes.try_into().unwrap());

            tracing::trace!("BTreeMap at {address:#x}, length: {length}");

            if length == 0 {
                return Ok(vec![]);
            }

            // Read the root field (Option<Root>)
            let root_addr = address + root_offset as u64;

            // Check if the Option is Some (non-zero discriminant)
            let discriminant_bytes = data_resolver.read_memory(root_addr, 8)?;
            let discriminant = u64::from_le_bytes(discriminant_bytes.try_into().unwrap());

            if discriminant == 0 {
                // None variant - empty map
                return Ok(vec![]);
            }

            // Read the root node pointer and height
            let node_ptr_addr = root_addr + root_layout.node_offset as u64;
            let height_addr = root_addr + root_layout.height_offset as u64;

            let node_ptr = data_resolver.read_address(node_ptr_addr)?;
            let height_bytes = data_resolver.read_memory(height_addr, 8)?;
            let height = usize::from_le_bytes(height_bytes.try_into().unwrap());

            tracing::trace!("BTreeMap root node: {node_ptr:#x}, height: {height}");

            // Traverse the tree starting from the root
            let mut entries = Vec::new();
            read_btree_node_entries(
                node_ptr,
                height,
                &def.key_type,
                &def.value_type,
                &node_layout,
                data_resolver,
                &mut entries,
            )?;

            Ok(entries)
        }
        MapVariant::IndexMap => {
            unimplemented!(
                "read_std_from_memory: MapVariant::IndexMap not implemented yet: {def:#?}"
            )
        }
    }
}

fn read_enum(
    db: &dyn Db,
    address: u64,
    enum_def: &EnumLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    tracing::trace!("read_enum {address:#x} {enum_def:#?}");

    let EnumLayout {
        name,
        variants,
        discriminant,
        ..
    } = enum_def;

    let disc_offset = discriminant.offset as u64;
    let disc_address = address + disc_offset;

    let explicit_disc = !matches!(discriminant.ty, rudy_types::DiscriminantType::Implicit);

    let disc_value = match &discriminant.ty {
        rudy_types::DiscriminantType::Int(int_def) => {
            let memory = data_resolver.read_memory(disc_address, int_def.size)?;

            match int_def.size {
                1 => i8::from_le_bytes(memory.try_into().unwrap()) as i128,
                2 => i16::from_le_bytes(memory.try_into().unwrap()) as i128,
                4 => i32::from_le_bytes(memory.try_into().unwrap()) as i128,
                8 => i64::from_le_bytes(memory.try_into().unwrap()) as i128,
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported IntDef size {} at address {address:#x}",
                        int_def.size
                    )
                }
            }
        }
        rudy_types::DiscriminantType::UnsignedInt(unsigned_int_def) => {
            let memory = data_resolver.read_memory(disc_address, unsigned_int_def.size)?;
            match unsigned_int_def.size {
                1 => u8::from_le_bytes(memory.try_into().unwrap()) as i128,
                2 => u16::from_le_bytes(memory.try_into().unwrap()) as i128,
                4 => u32::from_le_bytes(memory.try_into().unwrap()) as i128,
                8 => u64::from_le_bytes(memory.try_into().unwrap()) as i128,
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported UnsignedIntDef size {} at address {address:#x}",
                        unsigned_int_def.size
                    )
                }
            }
        }
        rudy_types::DiscriminantType::Implicit => {
            // I guess we'll just read 4 bytes and see what happens?
            let memory = data_resolver.read_memory(disc_address + disc_offset, 4)?;
            i32::from_le_bytes(memory.try_into().unwrap()) as i128
        }
    };

    tracing::trace!("read_enum: at {disc_address:#x}, discriminant value: {disc_value}");

    let maybe_variant = variants.iter().enumerate().find(|(i, v)| {
        if explicit_disc {
            // explicit discriminant, check against the value

            v.discriminant == Some(disc_value)
        } else {
            // otherwise we'll assume the discriminant is just the index
            *i as i128 == disc_value
        }
    });

    let matching_variant = match (maybe_variant, explicit_disc) {
        (Some((_, v)), _) => v,
        (None, true) => {
            // if we have _no_ discriminant, we assume this is the
            // fancy niche optimization that Rust does to pack in values
            variants.iter().find(|v| v.discriminant.is_none()).with_context(|| {
                format!(
                    "read_enum: No matching variant found for discriminant value {disc_value} in {name} with explicit discriminant"
                )
            })?
        }
        (None, false) => {
            anyhow::bail!(
                "read_enum: No matching variant found for discriminant value {disc_value} in {name} with implicit discriminant"
            )
        }
    };

    tracing::trace!("found matching variant: {matching_variant:#?}");

    // read the inner value
    let inner = read_from_memory(db, address, &matching_variant.layout, data_resolver)?;

    // re-format the value based on what we get back
    Ok(match inner {
        Value::Struct { fields, .. } => {
            if fields.is_empty() {
                // unit variant
                Value::Scalar {
                    ty: format!("{name}::{}", matching_variant.name),
                    value: matching_variant.name.to_string(),
                }
            } else if fields.keys().all(|k| k.starts_with("__")) {
                // anonymous struct, likely a tuple variant
                let values: Vec<_> = fields.into_values().collect();
                Value::Tuple {
                    ty: format!("{name}::{}", matching_variant.name),
                    entries: values,
                }
            } else {
                // struct variant
                Value::Struct {
                    ty: format!("{name}::{}", matching_variant.name),
                    fields,
                }
            }
        }
        v => {
            // unexpected, but we can at least return it
            tracing::error!("read_enum: Expected a struct for enum variant, got: {v:#?}");
            v
        }
    })
}

fn read_c_enum(
    address: u64,
    c_enum_def: &CEnumLayout,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    let CEnumLayout {
        name,
        discriminant_type,
        variants,
        ..
    } = c_enum_def;

    let disc_value = match discriminant_type {
        rudy_types::DiscriminantType::Int(int_def) => {
            let memory = data_resolver.read_memory(address, int_def.size)?;

            match int_def.size {
                1 => i8::from_le_bytes(memory.try_into().unwrap()) as i128,
                2 => i16::from_le_bytes(memory.try_into().unwrap()) as i128,
                4 => i32::from_le_bytes(memory.try_into().unwrap()) as i128,
                8 => i64::from_le_bytes(memory.try_into().unwrap()) as i128,
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported IntDef size {} at address {address:#x}",
                        int_def.size
                    )
                }
            }
        }
        rudy_types::DiscriminantType::UnsignedInt(unsigned_int_def) => {
            let memory = data_resolver.read_memory(address, unsigned_int_def.size)?;
            match unsigned_int_def.size {
                1 => u8::from_le_bytes(memory.try_into().unwrap()) as i128,
                2 => u16::from_le_bytes(memory.try_into().unwrap()) as i128,
                4 => u32::from_le_bytes(memory.try_into().unwrap()) as i128,
                8 => u64::from_le_bytes(memory.try_into().unwrap()) as i128,
                _ => {
                    anyhow::bail!(
                        "read_primitive_from_memory: unsupported UnsignedIntDef size {} at address {address:#x}",
                        unsigned_int_def.size
                    )
                }
            }
        }
        rudy_types::DiscriminantType::Implicit => {
            anyhow::bail!("read_c_enum: Implicit discriminant type is not supported yet")
        }
    };

    let matching_variant = variants
        .iter()
        .find_map(|v| (v.value == disc_value).then_some(&v.name))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "read_c_enum: No matching variant found for discriminant value {disc_value} in {name}"
            )
        })?;

    Ok(Value::Scalar {
        ty: format!("{name}::{matching_variant}"),
        value: disc_value.to_string(),
    })
}

pub fn read_from_memory(
    db: &dyn Db,
    address: u64,
    ty: &DieTypeDefinition,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    tracing::trace!("read_from_memory {address:#x} {}", ty.display_name());
    match ty.layout.as_ref() {
        Layout::Primitive(primitive_def) => {
            read_primitive_from_memory(db, address, primitive_def, data_resolver)
        }
        Layout::Struct(struct_def) => {
            let mut fields = BTreeMap::new();
            for field in &struct_def.fields {
                let field_name = &field.name;
                let field_ty = &field.ty;
                let field_address = address + (field.offset as u64);
                // Return a pointer instead of recursively reading
                let field_value = Value::Pointer(TypedPointer {
                    address: field_address,
                    type_def: field_ty.clone(),
                });
                fields.insert(field_name.to_string(), field_value);
            }
            Ok(Value::Struct {
                ty: struct_def.name.clone(),
                fields,
            })
        }
        Layout::Std(std_def) => read_std_from_memory(db, address, std_def, data_resolver),
        Layout::Enum(enum_def) => read_enum(db, address, enum_def, data_resolver),
        Layout::CEnum(c_enum_def) => read_c_enum(address, c_enum_def, data_resolver),
        Layout::Alias { .. } => {
            // For aliases, we'll resolve the underlying type and read that
            let underlying_type = rudy_dwarf::types::resolve_type_offset(db, ty.location)?;
            // now read the memory itself
            read_from_memory(db, address, &underlying_type, data_resolver)
        }
    }
}

/// Extract pointer, length, and capacity from a Vec Value
pub fn extract_vec_info(
    base_address: u64,
    def: &VecLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<(u64, usize)> {
    let VecLayout {
        length_offset,
        data_ptr_offset,
        ..
    } = def;
    let length = data_resolver
        .read_memory(base_address + *length_offset as u64, 8)?
        .try_into()
        .map(usize::from_le_bytes)
        .map_err(|_| {
            anyhow::anyhow!("Failed to read length for Vec at address {base_address:#x}")
        })?;
    tracing::trace!("Vec length: {length}");
    let address = data_resolver
        .read_address(base_address + *data_ptr_offset as u64)
        .with_context(|| {
            format!(
                "Failed to read Vec data pointer at {:#x}",
                base_address + *data_ptr_offset as u64
            )
        })?;
    Ok((address, length))
}

fn read_primitive_from_memory(
    db: &dyn Db,
    address: u64,
    def: &PrimitiveLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    let value = match def {
        PrimitiveLayout::Bool(_) => {
            let memory = data_resolver.read_memory(address, 1)?;
            let bool_value = memory[0] != 0;
            Value::Scalar {
                ty: "bool".to_string(),
                value: bool_value.to_string(),
            }
        }
        PrimitiveLayout::Char(()) => {
            let memory = data_resolver.read_memory(address, 4)?;
            let char_value = char::from_u32(u32::from_le_bytes(memory.try_into().unwrap()))
                .ok_or_else(|| anyhow::anyhow!("Invalid char value at address {address:#x}"))?;
            Value::Scalar {
                ty: "char".to_string(),
                value: format!("'{char_value}'"),
            }
        }
        PrimitiveLayout::Function(function_def) => Value::Scalar {
            ty: function_def.display_name(),
            value: format!("fn at {address:#x}"),
        },
        PrimitiveLayout::Array(ArrayLayout {
            element_type,
            length,
        }) => {
            let element_type = resolve_alias(db, element_type)?;
            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;

            let mut address = address;
            let mut values = Vec::with_capacity(*length);
            for _ in 0..*length {
                // Return pointers instead of recursively reading
                let value = Value::Pointer(TypedPointer {
                    address,
                    type_def: element_type.clone(),
                });
                values.push(value);
                address += element_size;
            }
            Value::Array {
                ty: format!("[{}; {length}]", element_type.display_name()),
                items: values,
            }
        }
        PrimitiveLayout::Pointer(PointerLayout { pointed_type, .. }) => {
            let address = data_resolver.read_address(address)?;
            read_from_memory(db, address, pointed_type, data_resolver)?.prefix_type("*")
        }
        PrimitiveLayout::Reference(ReferenceLayout { pointed_type, .. }) => {
            let address = data_resolver.read_address(address)?;
            read_from_memory(db, address, pointed_type, data_resolver)?.prefix_type("&")
        }
        PrimitiveLayout::Slice(SliceLayout {
            element_type,
            data_ptr_offset,
            length_offset,
        }) => {
            let length = address + *length_offset as u64;
            let length_bytes = data_resolver.read_memory(length, 8)?;
            let length = u64::from_le_bytes(length_bytes.try_into().unwrap());
            tracing::trace!("length: {length}");

            let element_type = resolve_alias(db, element_type)?;

            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;
            let data_ptr_address = data_resolver.read_address(address + *data_ptr_offset as u64)?;
            let mut current_addr = data_ptr_address;
            let mut values = Vec::with_capacity(length as usize);
            for _ in 0..length {
                // Return pointers instead of recursively reading
                let value = Value::Pointer(TypedPointer {
                    address: current_addr,
                    type_def: element_type.clone(),
                });
                values.push(value);
                current_addr += element_size;
            }
            Value::Array {
                ty: format!("&[{}]", element_type.display_name()),
                items: values,
            }
        }
        PrimitiveLayout::StrSlice(StrSliceLayout {
            data_ptr_offset,
            length_offset,
        }) => {
            let data_ptr = address + *data_ptr_offset as u64;
            let length = address + *length_offset as u64;

            let data_address = data_resolver.read_address(data_ptr)?;
            let memory = data_resolver.read_memory(length, 8)?;
            let length = u64::from_le_bytes(memory.try_into().unwrap());
            tracing::trace!("length: {length}");

            let memory = data_resolver.read_memory(data_address, length as usize)?;
            let string_value = String::from_utf8_lossy(&memory).to_string();
            Value::Scalar {
                ty: "str".to_string(),
                value: format!("\"{string_value}\""),
            }
        }
        PrimitiveLayout::UnsignedInt(unsigned_int_def) => {
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
            Value::Scalar {
                ty: unsigned_int_def.display_name(),
                value: num_string,
            }
        }
        PrimitiveLayout::Float(float_def) => {
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
            Value::Scalar {
                ty: format!("f{}", float_def.size * 8),
                value: num_string,
            }
        }
        PrimitiveLayout::Int(int_def) => {
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
            Value::Scalar {
                ty: int_def.display_name(),
                value: num_string,
            }
        }
        PrimitiveLayout::Never(_) => {
            // The Never type is a zero-sized type, so we return a placeholder value.
            Value::Scalar {
                ty: "Never".to_string(),
                value: "unreachable".to_string(),
            }
        }
        PrimitiveLayout::Str(()) => {
            unimplemented!("read_primitive_from_memory: bare `str` is not supported yet");
        }
        PrimitiveLayout::Tuple(tuple_def) => {
            unimplemented!(
                "read_primitive_from_memory: TupleDef not implemented yet: {tuple_def:#?}"
            );
        }
        PrimitiveLayout::Unit(_) => {
            // The Unit type is a zero-sized type, so we return a placeholder value.
            Value::Scalar {
                ty: "()".to_string(),
                value: "()".to_string(),
            }
        }
    };

    Ok(value)
}

fn resolve_alias(db: &dyn Db, def: &DieTypeDefinition) -> Result<DieTypeDefinition> {
    if let Layout::Alias { name } = def.layout.as_ref() {
        rudy_dwarf::types::resolve_type_offset(db, def.location)
            .with_context(|| format!("Failed to resolve alias for {name}"))
    } else {
        Ok(def.clone())
    }
}

fn read_option_from_memory(
    db: &dyn Db,
    address: u64,
    opt_def: &OptionLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    let OptionLayout {
        discriminant,
        some_offset,
        some_type,
        ..
    } = opt_def;

    let first_byte = data_resolver.read_memory(
        address + opt_def.discriminant.offset as u64,
        discriminant.size(),
    )?;
    Ok(if first_byte[0] == 0 {
        Value::Scalar {
            ty: some_type.display_name(),
            value: "None".to_string(),
        }
    } else {
        tracing::debug!("Found Some variant at {address:#x}: {some_type:#?}");
        // we have a `Some` variant
        // we should get the address of the inner value
        read_from_memory(db, address + *some_offset as u64, some_type, data_resolver)?
    }
    .wrap_type("Option"))
}

fn read_std_from_memory(
    db: &dyn Db,
    address: u64,
    def: &StdLayout<Die>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Value> {
    let value = match def {
        StdLayout::Option(enum_def) => {
            tracing::trace!("reading Option at {address:#x}");
            read_option_from_memory(db, address, enum_def, data_resolver)?
        }
        StdLayout::Vec(
            v @ VecLayout {
                length_offset,
                data_ptr_offset,
                inner_type,
                ..
            },
        ) => {
            tracing::trace!(
                "reading Vec at {address:#x}, length_offset: {length_offset:#x}, data_ptr_offset: {data_ptr_offset:#x}",
            );
            let element_type = resolve_alias(db, inner_type)?;

            let element_size = element_type.size().with_context(|| {
                format!(
                    "inner type: {} has unknown size",
                    element_type.display_name()
                )
            })? as u64;

            let (mut address, length) = extract_vec_info(address, v, data_resolver)?;
            let mut values = Vec::with_capacity(length);
            tracing::trace!("reading Vec data at {address:#016x}");
            for _ in 0..length {
                // Return pointers instead of recursively reading
                let value = Value::Pointer(TypedPointer {
                    address,
                    type_def: element_type.clone(),
                });
                values.push(value);
                address += element_size;
            }
            Value::Array {
                ty: format!("Vec<{}>", element_type.display_name()),
                items: values,
            }
        }
        StdLayout::String(s) => {
            let v = &s.0;
            tracing::trace!(
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
            tracing::trace!("String length: {length}");
            // read the bytes from the data pointer
            let data_address = address + s.0.data_ptr_offset as u64;
            let data = data_resolver.read_address(data_address).with_context(|| {
                format!("Failed to read String data pointer at {data_address:#x}")
            })?;
            tracing::trace!("reading String data at {data:#016x}");
            let bytes = data_resolver.read_memory(data, length)?;
            let value = String::from_utf8_lossy(&bytes).to_string();
            Value::Scalar {
                ty: "String".to_string(),
                value: format!("\"{value}\""),
            }
        }
        StdLayout::Map(def) => {
            let entries = read_map_entries(address, def, data_resolver)?
                .into_iter()
                .map(|(key, value)| Ok((Value::Pointer(key), Value::Pointer(value))))
                .collect::<Result<Vec<_>>>()?;
            Value::Map {
                ty: def.display_name(),
                entries,
            }
        }
        StdLayout::SmartPtr(s) => match s.variant {
            SmartPtrVariant::Mutex | SmartPtrVariant::RefCell => {
                let inner_type = s.inner_type.clone();
                let address = address + s.inner_ptr_offset as u64;
                read_from_memory(db, address, &inner_type, data_resolver)?
                    .wrap_type(s.variant.name())
            }
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
                unimplemented!("read_std_from_memory: SmartPtrVariant not implemented yet: {s:#?}")
            }
        },
        StdLayout::Result(result_def) => {
            unimplemented!("read_std_from_memory: ResultDef not implemented yet: {result_def:#?}")
        }
    };

    Ok(value)
}

/// Recursively read entries from a BTree node following the algorithm from the Python code
#[allow(clippy::too_many_arguments)]
fn read_btree_node_entries(
    node_ptr: u64,
    height: usize,
    key_type: &DieTypeDefinition,
    value_type: &DieTypeDefinition,
    node_layout: &BTreeNodeLayout,
    data_resolver: &dyn crate::DataResolver,
    entries: &mut Vec<(TypedPointer, TypedPointer)>,
) -> Result<()> {
    tracing::trace!("read_btree_node_entries at {node_ptr:#x}, height: {height}");

    // Read the node length (number of key-value pairs in this node)
    let len_addr = node_ptr + node_layout.len_offset as u64;
    let len_bytes = data_resolver.read_memory(len_addr, 2)?; // len is u16
    let len = u16::from_le_bytes(len_bytes.try_into().unwrap()) as usize;

    tracing::trace!("Node at {node_ptr:#x} has {len} entries");

    // Calculate sizes for key and value types
    let key_size = key_type
        .size()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine key size for BTreeMap"))?;
    let value_size = value_type
        .size()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine value size for BTreeMap"))?;

    // Get the base addresses for keys and values arrays
    let keys_addr = node_ptr + node_layout.keys_offset as u64;
    let vals_addr = node_ptr + node_layout.vals_offset as u64;

    // If this is an internal node (height > 0), we need to traverse edges
    let edges_addr = if height > 0 {
        Some(node_ptr + node_layout.edges_offset as u64)
    } else {
        None
    };

    // For each index from 0 to len (inclusive for edges)
    for i in 0..=len {
        // If this is an internal node, traverse the edge before processing the key/value
        if let Some(edges_base) = edges_addr {
            if i <= len {
                // Read the edge pointer (edges[i])
                // Edges are MaybeUninit<NodePtr>, so we need to handle the pointer size
                let edge_addr = edges_base + (i * 8) as u64; // Assume 8-byte pointers
                let edge_ptr = data_resolver.read_address(edge_addr)?;

                if edge_ptr != 0 {
                    // Recursively process the child node
                    read_btree_node_entries(
                        edge_ptr,
                        height - 1,
                        key_type,
                        value_type,
                        node_layout,
                        data_resolver,
                        entries,
                    )?;
                }
            }
        }

        // Process the key-value pair (only if i < len)
        if i < len {
            // Read key and value from the MaybeUninit arrays
            // Skip zero-sized types as per the Python code
            if key_size > 0 && value_size > 0 {
                let key_addr = keys_addr + (i * key_size) as u64;
                let value_addr = vals_addr + (i * value_size) as u64;

                let key_ptr = TypedPointer {
                    address: key_addr,
                    type_def: key_type.clone(),
                };

                let value_ptr = TypedPointer {
                    address: value_addr,
                    type_def: value_type.clone(),
                };

                entries.push((key_ptr, value_ptr));
            }
        }
    }

    Ok(())
}
