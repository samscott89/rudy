use std::{collections::BTreeMap, fmt, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use rudy_dwarf::{
    DebugFile, Die, SourceFile, function::resolve_function_variables, types::resolve_type_offset,
};
use rudy_types::{PrimitiveLayout, StdLayout, TypeLayout};

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
    debug_files: Vec<rudy_dwarf::DebugFile>,
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
        let (binary, debug_files) = db
            .analyze_file(binary_path.to_owned())
            .with_context(|| format!("Failed to analyze binary file: {}", binary_path.display()))?;

        let pb = Self {
            db,
            binary,
            debug_files,
        };

        // TODO(Sam): set up a file watcher if the binary and/or debug files change

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
    /// if let Some(location) = debug_info.address_to_line(0x12345) {
    ///     println!("Address 0x12345 is at {}:{}", location.file, location.line);
    /// }
    /// ```
    pub fn address_to_line(&self, address: u64) -> Option<ResolvedLocation> {
        self.resolve_address_to_location(address).unwrap()
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
    /// if let Some(func) = debug_info.resolve_function("main").unwrap() {
    ///     println!("Function 'main' is at address {:#x}", func.address);
    /// }
    /// ```
    pub fn resolve_function(&self, function: &str) -> Result<Option<ResolvedFunction>> {
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
            params: params
                .params(self.db)
                .into_iter()
                .enumerate()
                .map(|(i, var)| crate::Variable {
                    name: var
                        .name(self.db)
                        .as_ref()
                        .map_or_else(|| format!("__{i}"), |s| s.to_string()),
                    ty: Some(crate::Type {
                        name: var.ty(self.db).display_name(),
                    }),
                    value: None,
                })
                .collect(),
        }))
    }

    /// Resolves a memory address to its source location with error handling.
    ///
    /// Similar to `address_to_line` but provides detailed error information.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to resolve
    ///
    /// # Returns
    ///
    /// The source location if found, or `None` if the address cannot be resolved
    pub fn resolve_address_to_location(
        &self,
        address: u64,
    ) -> Result<Option<crate::ResolvedLocation>> {
        let db = self.db;
        let Some((name, loc)) = lookup_address(db, self.binary, address) else {
            tracing::debug!("no function found for address {address:#x}");
            return Ok(None);
        };

        Ok(Some(crate::ResolvedLocation {
            function: name.to_string(),
            file: loc.file(db).path_str(db).to_string(),
            line: loc.line(db),
        }))
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
    /// if let Some(addr) = debug_info.resolve_position("src/main.rs", 42, None).unwrap() {
    ///     println!("Line 42 of src/main.rs is at address {:#x}", addr.address);
    /// }
    /// ```
    pub fn resolve_position(
        &self,
        file: &str,
        line: u64,
        column: Option<u64>,
    ) -> Result<Option<crate::ResolvedAddress>> {
        let index = crate::index::debug_index(self.db, self.binary);

        let path = PathBuf::from(file.to_string());
        let source_file = SourceFile::new(self.db, path);

        let file = if index.source_to_file(self.db).contains_key(&source_file) {
            // already indexed file, so we can use it directly
            source_file
        } else {
            // otherwise, we need to find the file in the index
            if let Some(source_file) = index
                .source_to_file(self.db)
                .keys()
                .find(|f| f.path(self.db).ends_with(file))
            {
                tracing::debug!(
                    "found file `{file}` in debug index as `{}`",
                    source_file.path_str(self.db)
                );
                *source_file
            } else {
                tracing::warn!("file `{file}` not found in debug index");
                return Ok(None);
            }
        };

        let query = rudy_dwarf::file::SourceLocation::new(self.db, file, line, column);
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
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
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
            .params(db)
            .into_iter()
            .find(|var| var.name(db).as_deref() == Some(name))
        {
            tracing::info!(
                "Found parameter {name} in function {function_name} with type: {}",
                param.ty(db).display_name()
            );
            return variable_info(db, fie.declaration_die, base_addr, param, data_resolver)
                .map(Some);
        }

        // Then check locals
        if let Some(local) = vars
            .locals(db)
            .into_iter()
            .find(|var| var.name(db).as_deref() == Some(name))
        {
            tracing::info!(
                "Found variable {name} in function {function_name} with type: {}",
                local.ty(db).display_name()
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
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
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
            .params(db)
            .into_iter()
            .map(|param| variable_info(db, fie.declaration_die, base_addr, param, data_resolver))
            .collect::<Result<Vec<_>>>()?;

        let locals = vars
            .locals(db)
            .into_iter()
            .filter(|var| {
                // for local variables, we want to make sure the variable
                // is defined before the current location
                var.location(db)
                    .is_some_and(|var_loc| loc.line(db) > var_loc.line(db))
            })
            .map(|local| variable_info(db, fie.declaration_die, base_addr, local, data_resolver))
            .collect::<Result<Vec<_>>>()?;

        // TODO: handle globals
        Ok((params, locals, vec![]))
    }

    /// Reads and formats a variable's value from its metadata.
    ///
    /// This method takes variable metadata (from `get_variable_at_pc` or `get_all_variables_at_pc`)
    /// and reads the actual value from memory, formatting it for display.
    ///
    /// # Arguments
    ///
    /// * `var_info` - Variable metadata containing address and type information
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// The formatted variable value
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rudy_db::{DebugDb, DebugInfo, DataResolver};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// if let Some(var_info) = debug_info.get_variable_at_pc(0x12345, "foo", &resolver).unwrap() {
    ///     let value = debug_info.read_variable(&var_info, &resolver).unwrap();
    ///     println!("Variable value: {:?}", value);
    /// }
    /// ```
    pub fn read_variable(
        &self,
        var_info: &crate::VariableInfo,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<crate::Value> {
        if let Some(address) = var_info.address {
            crate::data::read_from_memory(
                self.db,
                address,
                &var_info.type_def,
                data_resolver,
                &var_info.debug_file,
            )
        } else {
            // Variable doesn't have a memory location (e.g., optimized out)
            Err(anyhow::anyhow!(
                "Variable '{}' has no memory location",
                var_info.name
            ))
        }
    }

    /// Resolves variables visible at a given memory address (legacy method).
    ///
    /// This method combines `get_all_variables_at_pc` and `read_variable` for convenience.
    /// For better performance or more control, prefer using the separate methods.
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
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// let (params, locals, globals) = debug_info
    ///     .resolve_variables_at_address(0x12345, &resolver)
    ///     .unwrap();
    /// println!("Found {} parameters, {} locals, {} globals",
    ///          params.len(), locals.len(), globals.len());
    /// ```
    pub fn resolve_variables_at_address(
        &self,
        address: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<(
        Vec<crate::Variable>,
        Vec<crate::Variable>,
        Vec<crate::Variable>,
    )> {
        // Use the new interface internally
        let (param_infos, local_infos, global_infos) =
            self.get_all_variables_at_pc(address, data_resolver)?;

        // Convert to Variables by reading values
        let params = param_infos
            .into_iter()
            .map(|info| {
                let value = if info.address.is_some() {
                    self.read_variable(&info, data_resolver).ok()
                } else {
                    None
                };
                crate::Variable {
                    name: info.name,
                    ty: Some(crate::Type {
                        name: info.type_def.display_name(),
                    }),
                    value,
                }
            })
            .collect();

        let locals = local_infos
            .into_iter()
            .map(|info| {
                let value = if info.address.is_some() {
                    self.read_variable(&info, data_resolver).ok()
                } else {
                    None
                };
                crate::Variable {
                    name: info.name,
                    ty: Some(crate::Type {
                        name: info.type_def.display_name(),
                    }),
                    value,
                }
            })
            .collect();

        let globals = global_infos
            .into_iter()
            .map(|info| {
                let value = if info.address.is_some() {
                    self.read_variable(&info, data_resolver).ok()
                } else {
                    None
                };
                crate::Variable {
                    name: info.name,
                    ty: Some(crate::Type {
                        name: info.type_def.display_name(),
                    }),
                    value,
                }
            })
            .collect();

        Ok((params, locals, globals))
    }

    /// Resolve a type by name in the debug information
    ///
    /// # Arguments
    ///
    /// * `type_name` - The name of the type to resolve (e.g., "String", "Vec", "TestPerson")
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
    /// if let Some((typedef, _)) = debug_info.resolve_type("String").unwrap() {
    ///     println!("Found String type: {}", typedef.display_name());
    /// }
    /// ```
    pub fn resolve_type(&self, type_name: &str) -> Result<Option<(TypeLayout, DebugFile)>> {
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
        let TypedPointer {
            address,
            type_def,
            debug_file,
        } = typed_pointer;
        crate::data::read_from_memory(self.db, *address, type_def, data_resolver, debug_file)
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
    /// if let Ok(field_info) = debug_info.get_field(var_info.address.unwrap(), &var_info.type_def, "name") {
    ///     println!("Field 'name' at address {:?}", field_info.address);
    /// }
    /// ```
    pub fn get_field(
        &self,
        base_address: u64,
        base_type: &TypeLayout,
        field_name: &str,
    ) -> Result<TypedPointer> {
        match base_type {
            TypeLayout::Struct(struct_def) => {
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

                // TODO: We should determine which debug file contains this type
                // For now, use the first debug file
                let debug_file = self
                    .debug_files
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("No debug files available"))?;
                Ok(TypedPointer {
                    address: field_address,
                    type_def: field.ty.clone(),
                    debug_file: *debug_file,
                })
            }
            TypeLayout::Enum(enum_def) => {
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
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// # let var_pointer: TypedPointer = unimplemented!();
    /// if let Ok(element_info) = debug_info.get_index_by_int(&var_pointer, 0, &resolver) {
    ///     println!("Element 0 at address {:?}", element_info.address);
    /// }
    /// ```
    pub fn get_index_by_int(
        &self,
        type_pointer: &TypedPointer,
        index: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<TypedPointer> {
        let TypedPointer {
            address: base_address,
            type_def: base_type,
            debug_file,
        } = type_pointer;
        let base_address = *base_address;

        match base_type.as_ref() {
            TypeLayout::Primitive(PrimitiveLayout::Array(array_def)) => {
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
                    debug_file: *debug_file,
                })
            }
            TypeLayout::Primitive(PrimitiveLayout::Slice(slice_def)) => {
                // Slice [T] - need to read the fat pointer to get actual data pointer and length
                let slice_value = crate::data::read_from_memory(
                    self.db,
                    base_address,
                    base_type,
                    data_resolver,
                    debug_file,
                )?;
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
                    debug_file: *debug_file,
                })
            }
            TypeLayout::Std(std_def) => {
                match std_def {
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

                        // TODO: We should determine which debug file contains this type
                        // For now, use the first debug file
                        let debug_file = self
                            .debug_files
                            .first()
                            .ok_or_else(|| anyhow::anyhow!("No debug files available"))?;
                        Ok(TypedPointer {
                            address: element_address,
                            type_def: vec_def.inner_type.clone(),
                            debug_file: *debug_file,
                        })
                    }
                    _ => Err(anyhow::anyhow!(
                        "Cannot index std type '{}' by integer",
                        base_type.display_name()
                    )),
                }
            }
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
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// # let var_info: VariableInfo = unimplemented!();
    /// # let key: Value = unimplemented!();
    /// if let Ok(value_info) = debug_info.get_index_by_value(var_info.address.unwrap(), &var_info.type_def, &key, &resolver) {
    ///     println!("Map value at address {:?}", value_info.address);
    /// }
    /// ```
    pub fn get_index_by_value(
        &self,
        base_address: u64,
        base_type: &TypeLayout,
        key: &crate::Value,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<TypedPointer> {
        match base_type {
            TypeLayout::Std(StdLayout::Map(map_def)) => {
                // For maps, we'll iterate through all key-value pairs
                // and return the variable info for the value that matches the key.

                // TODO: We should determine which debug file contains this type
                // For now, use the first debug file
                let debug_file = self
                    .debug_files
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("No debug files available"))?;
                let map_entries = crate::data::read_map_entries(
                    base_address,
                    map_def,
                    data_resolver,
                    debug_file,
                )?;

                for (k, v) in map_entries {
                    let map_key = crate::data::read_from_memory(
                        self.db,
                        k.address,
                        &k.type_def,
                        data_resolver,
                        &k.debug_file,
                    )?;

                    if values_equal(key, &map_key) {
                        return Ok(TypedPointer {
                            address: v.address,
                            type_def: v.type_def.clone(),
                            debug_file: v.debug_file,
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
        crate::function_discovery::discover_all_methods_for_pointer(
            self.db,
            self.binary,
            typed_pointer,
        )
    }
    // pub fn discover_methods_for_type(
    //     &self,
    //     target_type: &TypeLayout,
    // ) -> Result<Vec<DiscoveredMethod>> {
    //     crate::function_discovery::discover_all_methods_for_type(self.db, self.binary, target_type)
    // }
}

fn variable_info<'db>(
    db: &'db dyn Db,
    function: Die<'db>,
    base_address: u64,
    var: rudy_dwarf::function::Variable<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::VariableInfo> {
    let die = var.origin(db);
    let location = rudy_dwarf::expressions::resolve_data_location(
        db,
        function,
        base_address,
        die,
        &crate::data::DataResolverExpressionContext(data_resolver),
    )?;

    tracing::debug!("variable info: {:?} at {:?}", var.name(db), location);

    let type_def = match var.ty(db) {
        TypeLayout::Alias(unresolved_type) => {
            // For type aliases, resolve the actual type
            let entry = Die::from_unresolved_entry(db, die.file(db), unresolved_type);
            resolve_type_offset(db, entry).context("Failed to resolve type alias")?
        }
        t => t.clone(),
    };

    Ok(crate::VariableInfo {
        name: var
            .name(db)
            .as_ref()
            .map_or_else(|| "_".to_string(), |s| s.to_string()),
        address: location,
        type_def: Arc::new(type_def),
        debug_file: die.file(db),
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
