use std::{collections::BTreeMap, fmt, path::PathBuf};

use anyhow::{Context, Result};
use rudy_dwarf::{
    Die, SourceFile, SymbolName,
    function::resolve_function_variables,
    types::{DieTypeDefinition, resolve_type_offset},
};
use rudy_types::{Layout, PrimitiveLayout, StdLayout};

use crate::{
    DiscoveredMethod, ResolvedLocation,
    database::Db,
    function_discovery::SymbolAnalysisResult,
    index,
    outputs::{ResolvedFunction, TypedPointer},
    query::{lookup_address, lookup_position},
};

/// Main interface for accessing debug information from binary files.
///
/// `DebugInfo` provides methods to resolve addresses to source locations,
/// look up function information, and inspect variables at runtime.
///
/// The struct holds a reference to the debug database and manages the
/// binary file and associated debug files.
#[derive(Clone)]
pub struct DebugInfo<'db> {
    pub(crate) binary: rudy_dwarf::Binary,
    pub(crate) db: &'db crate::database::DebugDatabaseImpl,
}

impl<'db> fmt::Debug for DebugInfo<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let db = self.db;
        salsa::attach(db, || {
            let index = crate::index::debug_index(db, self.binary);

            f.debug_struct("DebugInfo")
                // .field("debug_files", &index.debug_files(db))
                .field("symbol_index", &index.symbol_index(db))
                .field("indexed_debug_files", &index.indexed_debug_files(db))
                .finish()
        })
    }
}

impl<'db> DebugInfo<'db> {
    /// Creates a new `DebugInfo` instance for analyzing a binary file.
    ///
    /// # Arguments
    ///
    /// * `db` - Reference to the debug database
    /// * `binary_path` - Path to the binary file to analyze
    ///
    /// # Returns
    ///
    /// A `DebugInfo` instance or an error if the binary cannot be loaded
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rudy_db::{DebugDb, DebugInfo};
    ///
    /// let db = DebugDb::new();
    /// let debug_info = DebugInfo::new(&db, "/path/to/binary").unwrap();
    /// ```
    pub fn new<P: AsRef<std::path::Path>>(
        db: &'db crate::database::DebugDatabaseImpl,
        binary_path: P,
    ) -> Result<Self> {
        let binary_path = binary_path.as_ref();
        let binary = db
            .load_binary(binary_path.to_owned())
            .with_context(|| format!("Failed to analyze binary file: {}", binary_path.display()))?;

        let pb = Self { db, binary };

        Ok(pb)
    }

    /// Resolves a memory address to its source location.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to resolve
    ///
    /// # Returns
    ///
    /// The source location if found, or `None` if the address cannot be resolved
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Ok(Some(location)) = debug_info.address_to_location(0x12345) {
    ///     println!("Address 0x12345 is at {}:{}", location.file, location.line);
    /// }
    /// ```
    pub fn address_to_location(&self, address: u64) -> Result<Option<ResolvedLocation>> {
        let db = self.db;
        let Some((name, loc)) = lookup_address(db, self.binary, address) else {
            tracing::debug!("no function found for address {address:#x}");
            return Ok(None);
        };

        Ok(Some(crate::ResolvedLocation {
            function: name.to_string(),
            file: loc.file.path_str().to_string(),
            line: loc.line,
        }))
    }

    /// Resolves a function name to its debug information.
    ///
    /// The function name can include module paths using `::` separators.
    ///
    /// # Arguments
    ///
    /// * `function` - The function name to resolve (e.g., "main" or "module::function")
    ///
    /// # Returns
    ///
    /// The resolved function information if found, or `None` if not found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Some(func) = debug_info.find_function_by_name("main").unwrap() {
    ///     println!("Function 'main' is at address {:#x}", func.address);
    /// }
    /// ```
    pub fn find_function_by_name(&self, function: &str) -> Result<Option<ResolvedFunction>> {
        let Some((name, _)) = index::find_closest_function(self.db, self.binary, function) else {
            tracing::debug!("no function found for {function}");
            return Ok(None);
        };

        let index = crate::index::debug_index(self.db, self.binary);
        let symbol_index = index.symbol_index(self.db);

        let symbol = symbol_index.get_function(&name).cloned().with_context(|| {
            tracing::debug!(?name, "{:#?}", symbol_index);
            "Failed to get base address for function"
        })?;

        let (_debug_file, fie) = index
            .get_function(self.db, &name)
            .ok_or_else(|| anyhow::anyhow!("Function not found in index: {name:?}"))?;

        let params = resolve_function_variables(self.db, fie)?;

        Ok(Some(ResolvedFunction {
            name: name.to_string(),
            address: symbol.address,
            size: fie
                .data(self.db)
                .address_range
                .map_or(0, |(start, end)| end - start),
            params: params
                .params
                .into_iter()
                .enumerate()
                .map(|(i, var)| crate::Variable {
                    name: var
                        .name
                        .as_ref()
                        .map_or_else(|| format!("__{i}"), |s| s.to_string()),
                    ty: var.ty.clone(),
                    value: None,
                })
                .collect(),
        }))
    }

    pub fn find_symbol_by_name(&self, symbol: &str) -> Result<Option<rudy_dwarf::symbols::Symbol>> {
        let index = crate::index::debug_index(self.db, self.binary);
        let symbol_index = index.symbol_index(self.db);

        let Some(symbols) = symbol_index.symbols.get(symbol) else {
            return Ok(None);
        };

        Ok(symbols.first_key_value().map(|(_, s)| s.clone()))
    }

    /// Resolves a source file position to a memory address.
    ///
    /// # Arguments
    ///
    /// * `file` - The source file path
    /// * `line` - The line number in the source file
    /// * `column` - Optional column number
    ///
    /// # Returns
    ///
    /// The memory address if the position can be resolved
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Some(addr) = debug_info.find_address_from_source_location("src/main.rs", 42, None).unwrap() {
    ///     println!("Line 42 of src/main.rs is at address {:#x}", addr.address);
    /// }
    /// ```
    pub fn find_address_from_source_location(
        &self,
        file: &str,
        line: u64,
        column: Option<u64>,
    ) -> Result<Option<crate::ResolvedAddress>> {
        let index = crate::index::debug_index(self.db, self.binary);

        let path = PathBuf::from(file.to_string());
        let source_file = SourceFile::new(path);

        let file = if index.source_to_file(self.db).contains_key(&source_file) {
            // already indexed file, so we can use it directly
            source_file
        } else {
            // otherwise, we need to find the file in the index
            if let Some(source_file) = index
                .source_to_file(self.db)
                .keys()
                .find(|f| f.path.ends_with(file))
            {
                tracing::debug!(
                    "found file `{file}` in debug index as `{}`",
                    source_file.path_str()
                );
                source_file.clone()
            } else {
                tracing::warn!("file `{file}` not found in debug index");
                return Ok(None);
            }
        };

        let query = rudy_dwarf::file::SourceLocation::new(file, line, column);
        let pos = lookup_position(self.db, self.binary, query);
        Ok(pos.map(|address| crate::ResolvedAddress { address }))
    }

    /// Gets metadata for a specific variable at a memory address without reading its value.
    ///
    /// This method is useful for expression evaluation where you need type information
    /// and memory addresses without immediately reading the value.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to inspect
    /// * `name` - The name of the variable to find
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// Variable metadata if found, or `None` if the variable is not found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, DataResolver};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver { }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// if let Some(var_info) = debug_info.get_variable_at_pc(0x12345, "foo", &resolver).unwrap() {
    ///     println!("Variable '{}' at address {:?}", var_info.name, var_info.address);
    /// }
    /// ```
    pub fn get_variable_at_pc(
        &self,
        address: u64,
        name: &str,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<Option<crate::VariableInfo>> {
        let db = self.db;
        let f = lookup_address(db, self.binary, address);

        let Some((function_name, _loc)) = f else {
            tracing::debug!("no function found for address {address:#x}");
            return Ok(None);
        };

        tracing::info!("Address {address:#08x} found in function {function_name}");

        let index = crate::index::debug_index(db, self.binary);
        let Some((_, fie)) = index.get_function(db, &function_name) else {
            tracing::debug!("no function found for {function_name}");
            return Ok(None);
        };

        let vars = resolve_function_variables(db, fie)?;

        let base_addr = crate::index::debug_index(db, self.binary)
            .symbol_index(db)
            .get_function(&function_name)
            .context("Failed to get base address for function")?
            .address;

        let fie = fie.data(db);
        // Check parameters first
        if let Some(param) = vars
            .params
            .into_iter()
            .find(|var| var.name.as_deref() == Some(name))
        {
            tracing::info!(
                "Found parameter {name} in function {function_name} with type: {}",
                param.ty.display_name()
            );
            return variable_info(db, fie.declaration_die, base_addr, param, data_resolver)
                .map(Some);
        }

        // Then check locals
        if let Some(local) = vars
            .locals
            .into_iter()
            .find(|var| var.name.as_deref() == Some(name))
        {
            tracing::info!(
                "Found variable {name} in function {function_name} with type: {}",
                local.ty.display_name()
            );
            return variable_info(db, fie.declaration_die, base_addr, local, data_resolver)
                .map(Some);
        }

        Ok(None)
    }

    /// Gets metadata for all variables at a memory address without reading their values.
    ///
    /// This method returns three categories of variables:
    /// - Function parameters
    /// - Local variables  
    /// - Global variables
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to inspect
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// A tuple of (parameters, locals, globals)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, DataResolver};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver { }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// let (params, locals, globals) = debug_info
    ///     .get_all_variables_at_pc(0x12345, &resolver)
    ///     .unwrap();
    /// println!("Found {} parameters, {} locals, {} globals",
    ///          params.len(), locals.len(), globals.len());
    /// ```
    pub fn get_all_variables_at_pc(
        &self,
        address: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<(
        Vec<crate::VariableInfo>,
        Vec<crate::VariableInfo>,
        Vec<crate::VariableInfo>,
    )> {
        let db = self.db;
        let f = lookup_address(db, self.binary, address);

        let Some((function_name, loc)) = f else {
            tracing::debug!("no function found for address {address:#x}");
            return Ok(Default::default());
        };

        let index = crate::index::debug_index(db, self.binary);
        let Some((_, fie)) = index.get_function(db, &function_name) else {
            tracing::debug!("no function found for {function_name}");
            return Ok(Default::default());
        };

        let vars = resolve_function_variables(db, fie)?;

        let base_addr = crate::index::debug_index(db, self.binary)
            .symbol_index(db)
            .get_function(&function_name)
            .context("Failed to get base address for function")?
            .address;

        let fie = fie.data(db);
        let params = vars
            .params
            .into_iter()
            .map(|param| variable_info(db, fie.declaration_die, base_addr, param, data_resolver))
            .collect::<Result<Vec<_>>>()?;

        let locals = vars
            .locals
            .into_iter()
            .filter(|var| {
                // for local variables, we want to make sure the variable
                // is defined before the current location
                var.location
                    .as_ref()
                    .is_some_and(|var_loc| loc.line > var_loc.line)
            })
            .map(|local| variable_info(db, fie.declaration_die, base_addr, local, data_resolver))
            .collect::<Result<Vec<_>>>()?;

        // TODO: handle globals
        Ok((params, locals, vec![]))
    }

    /// Resolve a type by name in the debug information
    ///
    /// Note: The type name _must_ be fully qualified, e.g., "alloc::string::String",
    /// and must include any generic parameters if applicable (e.g., "alloc::vec::Vec<u8>").
    ///
    /// Where possible it's better to find a variable at an address, and then
    /// get the type of the variable.
    ///
    /// # Arguments
    ///
    /// * `type_name` - The name of the type to resolve
    ///
    /// # Returns
    ///
    /// The resolved type definition if found, or `None` if the type cannot be found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Some(typedef) = debug_info.lookup_type_by_name("alloc::string::String").unwrap() {
    ///     println!("Found String type: {}", typedef.display_name());
    /// }
    /// ```
    pub fn lookup_type_by_name(&self, type_name: &str) -> Result<Option<DieTypeDefinition>> {
        crate::index::resolve_type(self.db, self.binary, type_name)
    }

    /// Read a value from memory using type information
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to read from
    /// * `typed_pointer` - TODO
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// The interpreted value from memory
    /// ```
    pub fn read_pointer(
        &self,
        typed_pointer: &TypedPointer,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<crate::Value> {
        let TypedPointer { address, type_def } = typed_pointer;
        crate::data::read_from_memory(self.db, *address, type_def, data_resolver)
    }

    /// Access a field of a struct/union/enum value
    ///
    /// # Arguments
    ///
    /// * `base_address` - Memory address of the base value
    /// * `base_type` - Type definition of the base value
    /// * `field_name` - Name of the field to access
    ///
    /// # Returns
    ///
    /// Variable information for the field if found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, VariableInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let var_info: VariableInfo = unimplemented!();
    /// if let Ok(field_info) = debug_info.get_struct_field(var_info.address.unwrap(), &var_info.type_def, "name") {
    ///     println!("Field 'name' at address {:?}", field_info.address);
    /// }
    /// ```
    pub fn get_struct_field(
        &self,
        base_address: u64,
        base_type: &DieTypeDefinition,
        field_name: &str,
    ) -> Result<TypedPointer> {
        match base_type.layout.as_ref() {
            Layout::Struct(struct_def) => {
                let field = struct_def
                    .fields
                    .iter()
                    .find(|f| f.name == field_name)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Field '{}' not found in struct '{}'",
                            field_name,
                            struct_def.name
                        )
                    })?;

                let field_address = base_address + field.offset as u64;
                Ok(TypedPointer {
                    address: field_address,
                    type_def: field.ty.clone(),
                })
            }
            Layout::Enum(enum_def) => {
                // For enums, field access might be variant data access
                // This is complex - for now return an error
                Err(anyhow::anyhow!(
                    "Enum field access not yet implemented for '{}'",
                    enum_def.name
                ))
            }
            _ => Err(anyhow::anyhow!(
                "Cannot access field '{}' on type '{}'",
                field_name,
                base_type.display_name()
            )),
        }
    }

    /// Index into an array/slice/vector by integer index
    ///
    /// # Arguments
    ///
    /// * `base_address` - Memory address of the base array/slice/vector
    /// * `base_type` - Type definition of the base value
    /// * `index` - Integer index to access
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// Variable information for the element at the given index
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, TypedPointer, VariableInfo, DataResolver};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver { }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// # let var_pointer: TypedPointer = unimplemented!();
    /// if let Ok(element_info) = debug_info.index_array_or_slice(&var_pointer, 0, &resolver) {
    ///     println!("Element 0 at address {:?}", element_info.address);
    /// }
    /// ```
    pub fn index_array_or_slice(
        &self,
        type_pointer: &TypedPointer,
        index: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<TypedPointer> {
        let TypedPointer {
            address: base_address,
            type_def: base_type,
        } = type_pointer;
        let base_address = *base_address;

        match base_type.layout.as_ref() {
            Layout::Primitive(PrimitiveLayout::Array(array_def)) => {
                // Fixed-size array [T; N]
                if index >= array_def.length as u64 {
                    return Err(anyhow::anyhow!(
                        "Index {} out of bounds for array of length {}",
                        index,
                        array_def.length
                    ));
                }

                let element_size = array_def.element_type.size().with_context(|| {
                    format!(
                        "Failed to get size for array element type '{}'",
                        array_def.element_type.display_name()
                    )
                })? as u64;
                let element_address = base_address + (index * element_size);
                Ok(TypedPointer {
                    address: element_address,
                    type_def: array_def.element_type.clone(),
                })
            }
            Layout::Primitive(PrimitiveLayout::Slice(slice_def)) => {
                // Slice [T] - need to read the fat pointer to get actual data pointer and length
                let slice_value =
                    crate::data::read_from_memory(self.db, base_address, base_type, data_resolver)?;
                let (data_ptr, slice_len) = extract_slice_info(&slice_value)?;

                if index >= slice_len {
                    return Err(anyhow::anyhow!(
                        "Index {} out of bounds for slice of length {}",
                        index,
                        slice_len
                    ));
                }

                let element_size = slice_def.element_type.size().with_context(|| {
                    format!(
                        "Failed to get size for slice element type '{}'",
                        slice_def.element_type.display_name()
                    )
                })? as u64;
                let element_address = data_ptr + (index * element_size);

                Ok(TypedPointer {
                    address: element_address,
                    type_def: slice_def.element_type.clone(),
                })
            }
            Layout::Std(std_def) => match std_def {
                StdLayout::Vec(vec_def) => {
                    let (data_ptr, vec_len) =
                        crate::data::extract_vec_info(base_address, vec_def, data_resolver)?;

                    if index as usize >= vec_len {
                        return Err(anyhow::anyhow!(
                            "Index {} out of bounds for Vec of length {}",
                            index,
                            vec_len
                        ));
                    }

                    let element_size = vec_def.inner_type.size().with_context(|| {
                        format!(
                            "Failed to get size for Vec element type '{}'",
                            vec_def.inner_type.display_name()
                        )
                    })? as u64;
                    let element_address = data_ptr + (index * element_size);
                    Ok(TypedPointer {
                        address: element_address,
                        type_def: vec_def.inner_type.clone(),
                    })
                }
                _ => Err(anyhow::anyhow!(
                    "Cannot index std type '{}' by integer",
                    base_type.display_name()
                )),
            },
            _ => Err(anyhow::anyhow!(
                "Cannot index type '{}' by integer",
                base_type.display_name()
            )),
        }
    }

    /// Index into a map/dictionary by value key
    ///
    /// # Arguments
    ///
    /// * `base_address` - Memory address of the base map
    /// * `base_type` - Type definition of the base map
    /// * `key` - Key value to look up
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// Variable information for the value at the given key
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, VariableInfo, DataResolver, Value};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver { }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// # let var_info: VariableInfo = unimplemented!();
    /// # let key: Value = unimplemented!();
    /// if let Ok(value_info) = debug_info.index_map(var_info.address.unwrap(), &var_info.type_def, &key, &resolver) {
    ///     println!("Map value at address {:?}", value_info.address);
    /// }
    /// ```
    pub fn index_map(
        &self,
        base_address: u64,
        base_type: &DieTypeDefinition,
        key: &crate::Value,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<TypedPointer> {
        match base_type.layout.as_ref() {
            Layout::Std(StdLayout::Map(map_def)) => {
                // For maps, we'll iterate through all key-value pairs
                // and return the variable info for the value that matches the key.
                let map_entries =
                    crate::data::read_map_entries(base_address, map_def, data_resolver)?;

                for (k, v) in map_entries {
                    let map_key = crate::data::read_from_memory(
                        self.db,
                        k.address,
                        &k.type_def,
                        data_resolver,
                    )?;

                    if values_equal(key, &map_key) {
                        return Ok(TypedPointer {
                            address: v.address,
                            type_def: v.type_def.clone(),
                        });
                    }
                }

                Err(anyhow::anyhow!(
                    "Key '{}' not found in map",
                    format_value_key(key)
                ))
            }
            _ => Err(anyhow::anyhow!(
                "Value-based indexing not supported for type '{}'",
                base_type.display_name()
            )),
        }
    }

    pub fn discover_all_methods(&self) -> Result<BTreeMap<String, Vec<DiscoveredMethod>>> {
        crate::function_discovery::discover_all_methods(self.db, self.binary)
    }

    pub fn discover_all_methods_debug(&self) -> Result<BTreeMap<String, SymbolAnalysisResult>> {
        crate::function_discovery::discover_all_functions_debug(self.db, self.binary)
    }

    pub fn discover_methods_for_pointer(
        &self,
        typed_pointer: &TypedPointer,
    ) -> Result<Vec<DiscoveredMethod>> {
        Ok(crate::function_discovery::discover_methods_for_type(
            self.db,
            self.binary,
            &typed_pointer.type_def,
        )?
        .into_iter()
        // filter out associated methods that are not self methods
        .filter(|m| m.self_type.is_some())
        .collect())
    }

    pub fn discover_methods_for_type(
        &self,
        type_def: &DieTypeDefinition,
    ) -> Result<Vec<DiscoveredMethod>> {
        crate::function_discovery::discover_methods_for_type(self.db, self.binary, type_def)
    }

    /// Discover functions in the binary that match a given pattern
    ///
    /// This method searches through all function symbols in the binary and returns
    /// functions that match the provided pattern. It supports:
    /// - Exact matches (e.g., "main")
    /// - Fuzzy matches (e.g., "calc" matching "calculate_sum")
    /// - Fully qualified names (e.g., "test_mod1::my_fn")
    ///
    /// # Arguments
    ///
    /// * `pattern` - The pattern to match against function names
    ///
    /// # Returns
    ///
    /// A vector of discovered functions sorted by match quality (exact matches first)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// // Find all functions containing "main"
    /// let functions = debug_info.discover_functions("main").unwrap();
    /// for func in functions {
    ///     println!("Found function: {} at address {:#x}", func.name, func.address);
    /// }
    /// ```
    pub fn discover_functions(&self, pattern: &str) -> Result<Vec<crate::DiscoveredFunction>> {
        let pattern = SymbolName::parse(pattern)
            .with_context(|| format!("Failed to parse function pattern: {pattern}"))?;
        crate::function_discovery::discover_functions(self.db, self.binary, &pattern)
    }

    /// Discover all functions in the binary
    ///
    /// Returns a map of function name to discovered function information.
    /// This includes both functions with debug information and those without.
    ///
    /// # Returns
    ///
    /// A map of function name to discovered function information
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// let all_functions = debug_info.discover_all_functions().unwrap();
    /// println!("Found {} functions in binary", all_functions.len());
    /// for (name, func) in all_functions {
    ///     println!("Function: {} -> {}", name, func.signature);
    /// }
    /// ```
    pub fn discover_all_functions(&self) -> Result<BTreeMap<String, crate::DiscoveredFunction>> {
        crate::function_discovery::discover_all_functions(self.db, self.binary)
    }

    /// Create a typed value in the target process based on the target type.
    ///
    /// This method uses DWARF type information to determine the correct conversion
    /// strategy for creating values that match function parameter types.
    ///
    /// # Arguments
    ///
    /// * `source_value` - The source value to convert (e.g., string literal, number)
    /// * `target_type` - The target type definition from DWARF
    /// * `data_resolver` - DataResolver for memory allocation and writing
    ///
    /// # Returns
    ///
    /// The address where the typed value was created in target memory
    pub fn create_typed_value(
        &self,
        source_value: &str,
        target_type: &DieTypeDefinition,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<u64> {
        match target_type.layout.as_ref() {
            Layout::Primitive(PrimitiveLayout::StrSlice(str_slice_layout)) => {
                // Create &str fat pointer using the actual layout
                self.create_str_slice_with_layout(source_value, str_slice_layout, data_resolver)
            }
            Layout::Std(StdLayout::String(string_layout)) => {
                // Create owned String using the actual layout
                self.create_owned_string_with_layout(source_value, string_layout, data_resolver)
            }
            _ => Err(anyhow::anyhow!(
                "Cannot convert string literal '{}' to type '{}'. Only &str and String are currently supported.",
                source_value,
                target_type.display_name()
            )),
        }
    }

    /// Create a Rust string slice (&str) in the target process using the actual layout
    fn create_str_slice_with_layout(
        &self,
        value: &str,
        layout: &rudy_types::StrSliceLayout,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<u64> {
        let bytes = value.as_bytes();
        let data_size = bytes.len();

        // Log the layout for debugging
        tracing::debug!(
            "Creating &str '{}' using layout: data_ptr_offset={}, length_offset={}",
            value,
            layout.data_ptr_offset,
            layout.length_offset
        );

        // Validate layout offsets are reasonable for a &str (should be 0 and 8 typically)
        if layout.data_ptr_offset > 16 || layout.length_offset > 16 {
            return Err(anyhow::anyhow!(
                "Invalid StrSliceLayout: data_ptr_offset={}, length_offset={}. Expected offsets <= 16.",
                layout.data_ptr_offset,
                layout.length_offset
            ));
        }

        // &str is typically 16 bytes (8-byte pointer + 8-byte length)
        let str_slice_size = 16;
        let total_size = data_size + str_slice_size;

        // Allocate memory for both the string data and the fat pointer
        let base_addr = data_resolver.allocate_memory(total_size)?;

        // Write the string data first
        let data_addr = base_addr;
        data_resolver.write_memory(data_addr, bytes)?;

        // Create the fat pointer at the end of the allocated memory using actual offsets
        let fat_ptr_addr = base_addr + data_size as u64;

        tracing::debug!(
            "Memory layout: base_addr={:#x}, data_addr={:#x}, fat_ptr_addr={:#x}, data_size={}",
            base_addr,
            data_addr,
            fat_ptr_addr,
            data_size
        );

        // Write the data pointer at the correct offset
        let data_ptr_addr = fat_ptr_addr + layout.data_ptr_offset as u64;
        tracing::debug!(
            "Writing data pointer {:#x} to address {:#x} (fat_ptr_addr + {})",
            data_addr,
            data_ptr_addr,
            layout.data_ptr_offset
        );
        data_resolver.write_memory(data_ptr_addr, &data_addr.to_le_bytes())?;

        // Write the length at the correct offset
        let length_addr = fat_ptr_addr + layout.length_offset as u64;
        tracing::debug!(
            "Writing length {} to address {:#x} (fat_ptr_addr + {})",
            data_size,
            length_addr,
            layout.length_offset
        );
        data_resolver.write_memory(length_addr, &(data_size as u64).to_le_bytes())?;

        // Validate the created &str by reading it back
        tracing::debug!(
            "Created &str at {:#x}: data_ptr at offset {}, length {} at offset {}",
            fat_ptr_addr,
            layout.data_ptr_offset,
            data_size,
            layout.length_offset
        );

        // Read back the pointer and length to validate
        let read_data_ptr_bytes = data_resolver.read_memory(data_ptr_addr, 8)?;
        let read_length_bytes = data_resolver.read_memory(length_addr, 8)?;
        let read_data_ptr = u64::from_le_bytes(read_data_ptr_bytes.try_into().unwrap());
        let read_length = u64::from_le_bytes(read_length_bytes.try_into().unwrap());

        tracing::debug!(
            "Validation: &str at {:#x} -> data_ptr={:#x}, length={}",
            fat_ptr_addr,
            read_data_ptr,
            read_length
        );

        // Sanity check the values
        if read_data_ptr != data_addr {
            return Err(anyhow::anyhow!(
                "Invalid &str: data pointer mismatch. Expected {:#x}, got {:#x}",
                data_addr,
                read_data_ptr
            ));
        }
        if read_length != data_size as u64 {
            return Err(anyhow::anyhow!(
                "Invalid &str: length mismatch. Expected {}, got {}",
                data_size,
                read_length
            ));
        }
        if read_length > 1024 * 1024 {
            // Sanity check: strings > 1MB are suspicious
            return Err(anyhow::anyhow!(
                "Invalid &str: length {} is suspiciously large (> 1MB)",
                read_length
            ));
        }

        // Final validation: read the actual string data to make sure it's correct
        let read_string_data = data_resolver.read_memory(read_data_ptr, read_length as usize)?;
        let read_string = String::from_utf8_lossy(&read_string_data);
        tracing::debug!(
            "Final validation: &str points to data '{}' (expected '{}')",
            read_string,
            value
        );

        if read_string != value {
            return Err(anyhow::anyhow!(
                "Invalid &str: string data mismatch. Expected '{}', got '{}'",
                value,
                read_string
            ));
        }

        Ok(fat_ptr_addr)
    }

    /// Create an owned String in the target process using the actual DWARF layout
    fn create_owned_string_with_layout(
        &self,
        value: &str,
        string_layout: &rudy_types::StringLayout<rudy_dwarf::Die>,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<u64> {
        use rudy_types::Layout;

        tracing::debug!("Creating owned String with layout for value: '{}'", value);

        let vec_layout = &string_layout.0; // StringLayout(VecLayout)
        let string_len = value.len();

        // Step 1: Allocate memory for the string content (heap data)
        let content_addr = data_resolver.allocate_memory(string_len)?;
        tracing::debug!("Allocated string content at: {:#x}", content_addr);

        // Step 2: Write the string bytes to the content memory
        data_resolver.write_memory(content_addr, value.as_bytes())?;
        tracing::debug!("Wrote {} bytes of string content", string_len);

        // Step 3: Calculate the size of the String struct from the layout
        let string_struct_size = Layout::Std(rudy_types::StdLayout::String(string_layout.clone()))
            .size()
            .ok_or_else(|| anyhow::anyhow!("Could not determine String struct size from layout"))?;

        // Step 4: Allocate memory for the String struct itself
        let string_addr = data_resolver.allocate_memory(string_struct_size)?;
        tracing::debug!(
            "Allocated String struct at: {:#x} (size: {} bytes)",
            string_addr,
            string_struct_size
        );

        // Step 5: Zero out the struct memory first
        let zero_bytes = vec![0u8; string_struct_size];
        data_resolver.write_memory(string_addr, &zero_bytes)?;

        // Step 6: Populate the String struct fields using the layout offsets

        // Write the length field (vec.len)
        let len_bytes = (string_len as u64).to_le_bytes();
        data_resolver.write_memory(string_addr + vec_layout.length_offset as u64, &len_bytes)?;
        tracing::debug!(
            "Set length field at offset {:#x} to {}",
            vec_layout.length_offset,
            string_len
        );

        // Write the data pointer field (vec.buf.inner.ptr.pointer)
        let ptr_bytes = content_addr.to_le_bytes();
        data_resolver.write_memory(string_addr + vec_layout.data_ptr_offset as u64, &ptr_bytes)?;
        tracing::debug!(
            "Set data pointer field at offset {:#x} to {:#x}",
            vec_layout.data_ptr_offset,
            content_addr
        );

        // Write the capacity field (vec.buf.inner.cap.__0)
        // For simplicity, set capacity equal to length
        let cap_bytes = (string_len as u64).to_le_bytes();
        data_resolver.write_memory(string_addr + vec_layout.capacity_offset as u64, &cap_bytes)?;
        tracing::debug!(
            "Set capacity field at offset {:#x} to {}",
            vec_layout.capacity_offset,
            string_len
        );

        // Note: Other fields (allocator, phantom data, etc.) are left as zero,
        // which should be appropriate for the Global allocator and PhantomData

        tracing::debug!(
            "Successfully created String at {:#x} pointing to data at {:#x}",
            string_addr,
            content_addr
        );

        Ok(string_addr)
    }
}

fn variable_info(
    db: &dyn Db,
    function: Die,
    base_address: u64,
    var: rudy_dwarf::function::Variable,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::VariableInfo> {
    let die = var.origin;
    let location = rudy_dwarf::expressions::resolve_data_location(
        db,
        function,
        base_address,
        die,
        &crate::data::DataResolverExpressionContext(data_resolver),
    )?;

    tracing::debug!("variable info: {:?} at {:?}", var.name, location);

    let ty = var.ty;
    let type_def = match ty.layout.as_ref() {
        Layout::Alias { .. } => {
            // For type aliases, resolve the actual type
            resolve_type_offset(db, ty.location).context("Failed to resolve type alias")?
        }
        _ => ty.clone(),
    };

    Ok(crate::VariableInfo {
        name: var
            .name
            .as_ref()
            .map_or_else(|| "_".to_string(), |s| s.to_string()),
        address: location,
        type_def,
    })
}

/// Extract pointer and length from a slice Value
fn extract_slice_info(slice_value: &crate::Value) -> Result<(u64, u64)> {
    match slice_value {
        crate::Value::Struct { fields, .. } => {
            // Look for ptr/data_ptr and len fields
            let ptr = fields
                .get("data_ptr")
                .or_else(|| fields.get("ptr"))
                .ok_or_else(|| anyhow::anyhow!("Slice missing data pointer field"))?;

            let len = fields
                .get("len")
                .or_else(|| fields.get("length"))
                .ok_or_else(|| anyhow::anyhow!("Slice missing length field"))?;

            let ptr_value = extract_numeric_value(ptr)?;
            let len_value = extract_numeric_value(len)?;

            Ok((ptr_value, len_value))
        }
        _ => Err(anyhow::anyhow!(
            "Expected struct representation for slice, got: {:?}",
            slice_value
        )),
    }
}

/// Extract a numeric value from a Value (handles various scalar representations)
fn extract_numeric_value(value: &crate::Value) -> Result<u64> {
    match value {
        crate::Value::Scalar { value, .. } => {
            // Try to parse as different number formats
            if let Ok(num) = value.parse::<u64>() {
                Ok(num)
            } else if let Some(hex_value) = value.strip_prefix("0x") {
                u64::from_str_radix(hex_value, 16)
                    .with_context(|| format!("Failed to parse hex value: {value}"))
            } else {
                Err(anyhow::anyhow!("Could not parse numeric value: {}", value))
            }
        }
        _ => Err(anyhow::anyhow!("Expected scalar value, got: {:?}", value)),
    }
}

/// Compare two Values for equality (approximate, for HashMap key matching)
fn values_equal(a: &crate::Value, b: &crate::Value) -> bool {
    match (a, b) {
        (crate::Value::Scalar { value: a_val, .. }, crate::Value::Scalar { value: b_val, .. }) => {
            // For strings, compare the actual string content (strip quotes if present)
            let a_clean = a_val.trim_matches('"');
            let b_clean = b_val.trim_matches('"');
            a_clean == b_clean
        }
        // For more complex types, could add more sophisticated comparison
        _ => false,
    }
}

/// Format a Value as a key for display purposes
fn format_value_key(value: &crate::Value) -> String {
    match value {
        crate::Value::Scalar { value, .. } => {
            // Strip quotes for cleaner display
            value.trim_matches('"').to_string()
        }
        _ => format!("{value:?}"),
    }
}
