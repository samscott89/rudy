use anyhow::{Context, Result};
use std::fmt;

use crate::{
    ResolvedLocation,
    database::{Db, Diagnostic, handle_diagnostics},
    dwarf::{self, Die, resolve_function_variables},
    index,
    outputs::ResolvedFunction,
    query::{lookup_address, lookup_closest_function, lookup_position},
    types::{Address, Position},
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
    pub(crate) binary: crate::file::Binary,
    debug_files: Vec<crate::file::DebugFile>,
    pub(crate) db: &'db crate::database::DebugDatabaseImpl,
}

impl<'db> fmt::Debug for DebugInfo<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::attach(self.db, || {
            f.debug_struct("DebugInfo")
                .field("debug_files", &self.debug_files)
                .field("index", crate::index::debug_index(self.db, self.binary))
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
    /// use rust_debuginfo::{DebugDb, DebugInfo};
    ///
    /// let db = DebugDb::new();
    /// let debug_info = DebugInfo::new(&db, "/path/to/binary").unwrap();
    /// ```
    pub fn new(db: &'db crate::database::DebugDatabaseImpl, binary_path: &str) -> Result<Self> {
        let (binary, debug_files) = db
            .analyze_file(binary_path)
            .with_context(|| format!("Failed to analyze binary file: {binary_path}"))?;

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
    /// # use rust_debuginfo::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Some(location) = debug_info.address_to_line(0x12345) {
    ///     println!("Address 0x12345 is at {}:{}", location.file, location.line);
    /// }
    /// ```
    pub fn address_to_line(&self, address: u64) -> Option<ResolvedLocation> {
        self.resolve_address_to_location(address).unwrap()
    }

    // pub fn resolve_function(&self, name: &str) -> Option<ResolvedAddress> {
    //     let f = self.lookup_function(name).unwrap()?;
    //     let address = f.relative_body_address(self.db);
    //     Some(ResolvedAddress { address })
    // }

    // pub fn get_source_lines(&self, _address: u64) -> Vec<String> {
    //     todo!()
    // }

    // pub fn resolve_variables_at_address(
    //     &self,
    //     address: u64,
    //     data_resolver: &dyn DataResolver,
    // ) -> (Vec<Variable>, Vec<Variable>, Vec<Variable>) {
    //     let (locals, params, globals) = self
    //         .db
    //         .resolve_variables_at_address(address, data_resolver)
    //         .unwrap();
    //     (locals, params, globals)
    // }

    // pub fn test_get_shape(&self) -> TypeDef<'_> {
    //     self.db.test_get_shape().unwrap()
    // }

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
    /// # use rust_debuginfo::{DebugDb, DebugInfo};
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
        let diagnostics: Vec<&Diagnostic> =
            index::find_closest_function::accumulated(self.db, self.binary, function);
        handle_diagnostics(&diagnostics)?;

        let debug_data = index::debug_index(self.db, self.binary).data(self.db);

        let base_address = debug_data
            .base_address
            .get(&name)
            .copied()
            .with_context(|| {
                tracing::debug!(?name, "{:#?}", debug_data.base_address);
                "Failed to get base address for function"
            })?;

        let (_debug_file, fie) = crate::index::debug_index(self.db, self.binary)
            .get_function(self.db, &name)
            .ok_or_else(|| anyhow::anyhow!("Function not found in index: {name:?}"))?;

        let function_die = fie.specification_die.unwrap_or(fie.declaration_die);
        let params = resolve_function_variables(self.db, function_die)?;
        let diagnostics: Vec<&Diagnostic> =
            dwarf::resolve_function_variables::accumulated(self.db, function_die);
        handle_diagnostics(&diagnostics)?;

        Ok(Some(ResolvedFunction {
            name: name.to_string(),
            address: base_address,
            params: params
                .params(self.db)
                .into_iter()
                .map(|var| crate::Variable {
                    name: var.name(self.db).to_string(),
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
        let address = Address::new(self.db, address);
        let loc = lookup_address(self.db, self.binary, address);
        let Some(name) = lookup_closest_function(self.db, self.binary, address) else {
            tracing::debug!(
                "no function found for address {:#x}",
                address.address(self.db)
            );
            return Ok(None);
        };

        Ok(loc.map(|loc| {
            let file = loc.file(self.db);
            crate::ResolvedLocation {
                function: name.to_string(),
                file: file.path(self.db).clone(),
                line: loc.line(self.db),
            }
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
    /// # use rust_debuginfo::{DebugDb, DebugInfo};
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
        let index = crate::index::debug_index(self.db, self.binary).data(self.db);

        let source_file = crate::file::SourceFile::new(self.db, file);

        let file = if index.source_to_file.contains_key(&source_file) {
            // already indexed file, so we can use it directly
            source_file
        } else {
            // otherwise, we need to find the file in the index
            if let Some(source_file) = index
                .source_to_file
                .keys()
                .find(|f| f.path(self.db).ends_with(&file))
            {
                tracing::debug!(
                    "found file `{file}` in debug index as `{}`",
                    source_file.path(self.db)
                );
                source_file.clone()
            } else {
                tracing::warn!("file `{file}` not found in debug index");
                return Ok(None);
            }
        };

        let query = Position::new(self.db, file, line, column);
        let pos = lookup_position(self.db, self.binary, query);
        Ok(pos.map(|address| crate::ResolvedAddress { address }))
    }

    /// Resolves variables visible at a given memory address.
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
    /// # use rust_debuginfo::{DebugDb, DebugInfo, DataResolver};
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
        let address = Address::new(self.db, address);
        let loc = lookup_address(self.db, self.binary, address);
        let f = lookup_closest_function(self.db, self.binary, address);
        let diagnostics: Vec<&Diagnostic> =
            lookup_closest_function::accumulated(self.db, self.binary, address);
        handle_diagnostics(&diagnostics)?;

        let Some(f) = f else {
            tracing::debug!(
                "no function found for address {:#x}",
                address.address(self.db)
            );
            return Ok(Default::default());
        };

        let index = crate::index::debug_index(self.db, self.binary);
        let Some((_, fie)) = index.get_function(self.db, &f) else {
            tracing::debug!("no function found for {f:?}");
            return Ok(Default::default());
        };

        let function_die = fie.specification_die.unwrap_or(fie.declaration_die);

        let vars = dwarf::resolve_function_variables(self.db, function_die)?;
        let diagnostics: Vec<&Diagnostic> =
            dwarf::resolve_function_variables::accumulated(self.db, function_die);
        handle_diagnostics(&diagnostics)?;

        let base_addr = *crate::index::debug_index(self.db, self.binary)
            .data(self.db)
            .base_address
            .get(&f)
            .context("Failed to get base address for function")?;

        let params = vars
            .params(self.db)
            .into_iter()
            .map(|param| {
                output_variable(
                    self.db,
                    fie.declaration_die,
                    base_addr,
                    param,
                    data_resolver,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let locals = vars
            .locals(self.db)
            .into_iter()
            .filter(|var| {
                // for local variables, we want to make sure the variable
                // is defined before the current location
                if let Some(loc) = loc {
                    loc.line(self.db) > var.line(self.db)
                } else {
                    // if we don't have a location, we assume the param is valid
                    true
                }
            })
            .map(|param| {
                output_variable(
                    self.db,
                    fie.declaration_die,
                    base_addr,
                    param,
                    data_resolver,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        // TODO: handle globals
        // let globals = vars
        //     .globals(self.db)
        //     .into_iter()
        //     .map(|(var, address)| output_global_variable(self.db, var, address, data_resolver))
        //     .collect::<Result<Vec<_>>>()?;
        Ok((params, locals, vec![]))
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
    /// # use rust_debuginfo::{DebugDb, DebugInfo};
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// if let Some(typedef) = debug_info.resolve_type("String").unwrap() {
    ///     println!("Found String type: {}", typedef.display_name());
    /// }
    /// ```
    pub fn resolve_type(&self, type_name: &str) -> Result<Option<crate::typedef::TypeDef>> {
        crate::index::resolve_type(self.db, self.binary, type_name)
    }

    /// Read a value from memory using type information
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address to read from
    /// * `typedef` - The type definition to use for interpreting the memory
    /// * `data_resolver` - Interface for reading memory and register values
    ///
    /// # Returns
    ///
    /// The interpreted value from memory
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rust_debuginfo::{DebugDb, DebugInfo, DataResolver};
    /// # struct MyResolver;
    /// # impl DataResolver for MyResolver {
    /// #     fn base_address(&self) -> u64 { 0 }
    /// #     fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> { Ok(vec![]) }
    /// #     fn get_registers(&self) -> anyhow::Result<Vec<u64>> { Ok(vec![]) }
    /// # }
    /// # let db = DebugDb::new();
    /// # let debug_info = DebugInfo::new(&db, "binary").unwrap();
    /// # let resolver = MyResolver;
    /// if let Some(typedef) = debug_info.resolve_type("String").unwrap() {
    ///     let value = debug_info.address_to_value(0x12345, &typedef, &resolver).unwrap();
    ///     println!("Value at address: {:?}", value);
    /// }
    /// ```
    pub fn address_to_value(
        &self,
        address: u64,
        typedef: &crate::typedef::TypeDef,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<crate::Value> {
        crate::data::read_from_memory(self.db, address, typedef, data_resolver)
    }
}

fn output_variable<'db>(
    db: &'db dyn Db,
    function: Die<'db>,
    base_address: u64,
    var: dwarf::Variable<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Variable> {
    let location =
        dwarf::resolve_data_location(db, function, base_address, var.origin(db), data_resolver)?;

    let value = if let Some(addr) = location {
        Some(crate::data::read_from_memory(
            db,
            addr,
            var.ty(db),
            data_resolver,
        )?)
    } else {
        None
    };

    tracing::debug!("variable: {} => {value:?}", var.name(db));

    Ok(crate::Variable {
        name: var.name(db).to_string(),
        ty: Some(crate::Type {
            name: var.ty(db).display_name(),
        }),
        value,
    })
}

#[allow(dead_code)]
fn output_global_variable<'db>(
    db: &'db dyn Db,
    var: dwarf::Variable<'db>,
    address: u64,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Variable> {
    let value = Some(crate::data::read_from_memory(
        db,
        address,
        var.ty(db),
        data_resolver,
    )?);

    tracing::debug!("variable: {} => {value:?}", var.name(db));

    Ok(crate::Variable {
        name: var.name(db).to_string(),
        ty: Some(crate::Type {
            name: var.ty(db).display_name(),
        }),
        value,
    })
}
