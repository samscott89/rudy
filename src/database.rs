//! Salsa database for debugging information
//!
//! This module provides a salsa database for debugging information.
//!
//! The core idea is to make debugging information (today: just DWARF)
//! available via salsa queries.
//!
//! The main benefit this provides is to make queries lazy, memoized, and
//! incremental.
//!
//! ## Approach
//!
//! The structure of debugging information in DWARF means that it's realtively
//! cheap to look things up once you know where they are, but finding it
//! requires walking/parsing multiple files/sections. Furthermore, there is
//! information that cannot be eagerly loaded, such as the location of variables
//! in memory since it depends on the current state of the program.
//!
//! Given all of this, we take a multi-pass approach:
//!
//! 1. Up-front, walk all the files to construct indexes into the debugging information
//!    that makes it quick to find the relevant files/sections. e.g.
//!     - Symbol -> compilation unit + offset
//!     - Source file -> compilation unit
//!     - Address -> relevant compilation units/sections
//!    This indexing happens on startup/initial loading of the files and
//!    only changes if the binary is recompiled (although we should be able
//!    to memoize anything looked up from the individual files).
//! 2. Lazily parse specific sections and memoize the results. This is only
//!    called whenever we need the information (e.g. when breaking on a line inside a function).
//!    But the results should be able to be memoized and ~never recomputed, even when stepping
//!    through a debug session
//! 3. Session-specific recomputed values. There are some things that we always need to recompute
//!    depending on the current session. E.g. when getting locations for variables when inside a
//!    function, or parsing stack frames. These will typically use a lot of cached/memoized
//!    intermediate results, but are unlikely to be themselves cached.
//!
//!
//! NOTE: today we don't actually have _any_ inputs. There is no incrementality since we're
//! assuming the debug information is static. However, in the future we may want incrementality
//! in via making the Binary file and all object files inputs -- this way if we recompile the
//! binary we can recompute which parts of the binary are the same and which are unchanged.

use std::sync::Arc;

use anyhow::{Context, Result};
use salsa::Accumulator;

use crate::data::Def;
use crate::dwarf;
use crate::file::{File, FileId, load_relocatable_file};
use crate::index;
use crate::query::{find_closest_match, lookup_address, lookup_closest_function, lookup_position};
use crate::types::{Address, FunctionIndexEntry, NameId, Position};

#[salsa::db]
pub trait Db: salsa::Database {
    fn binary_file(&self) -> &File;

    fn report_info(&self, message: String) {
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Info,
        }
        .accumulate(self);
    }
    fn report_warning(&self, message: String) {
        tracing::warn!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Warning,
        }
        .accumulate(self);
    }
    fn report_critical(&self, message: String) {
        tracing::error!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Critical,
        }
        .accumulate(self);
    }
    fn report_error(&self, message: String) {
        tracing::error!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Error,
        }
        .accumulate(self);
    }

    fn upcast(&self) -> &dyn Db;

    fn get_file<'db>(&'db self, file: FileId<'db>) -> Option<&'db File> {
        if file.relocatable(self) {
            let file = load_relocatable_file(self.upcast(), file)?;
            let file = file.file(self);
            if file.error().is_some() {
                None
            } else {
                Some(file)
            }
        } else {
            Some(self.binary_file())
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum DiagnosticSeverity {
    /// Errors that we never expect to see
    /// and imply an internal error.
    Critical,
    Error,
    Warning,
    Info,
}

#[salsa::accumulator]
struct Diagnostic {
    pub message: String,
    pub severity: DiagnosticSeverity,
}

#[salsa::db]
#[derive(Clone)]
pub struct DebugDatabaseImpl {
    storage: salsa::Storage<Self>,
    binary_file: Arc<File>,
}

fn handle_diagnostics(diagnostics: &[&Diagnostic]) -> Result<()> {
    let mut err = None;
    for d in diagnostics {
        match d.severity {
            DiagnosticSeverity::Critical => {
                if err.is_some() {
                    tracing::error!("Critical error: {}", d.message);
                } else {
                    err = Some(anyhow::anyhow!("Critical error: {}", d.message));
                }
            }
            DiagnosticSeverity::Error => {
                if err.is_some() {
                    tracing::error!("Error: {}", d.message);
                } else {
                    err = Some(anyhow::anyhow!("Error: {}", d.message));
                }
            }
            DiagnosticSeverity::Warning => {
                tracing::warn!("Warning: {}", d.message);
            }
            DiagnosticSeverity::Info => {
                tracing::info!("Info: {}", d.message);
            }
        }
    }

    if let Some(e) = err { Err(e) } else { Ok(()) }
}

#[salsa::tracked]
fn initialize<'db>(db: &'db dyn Db) {
    // Generate the index on startup to save time
    let _ = index::index(db);
}

impl DebugDatabaseImpl {
    pub fn new(binary_file: &str) -> Result<Self> {
        let lof = File::new(binary_file, None);
        if let Some(e) = lof.error() {
            return Err(anyhow::anyhow!("{e}"))
                .with_context(|| format!("Failed to load binary file: {binary_file}"));
        }

        let db = Self {
            storage: salsa::Storage::default(),
            binary_file: Arc::new(lof),
        };
        initialize(&db);
        Ok(db)
    }

    pub fn lookup_function(&self, function: &str) -> Result<Option<dwarf::Function>> {
        let mut split: Vec<String> = function.split("::").map(|s| s.to_owned()).collect();
        let name = split.pop().unwrap_or_else(|| {
            self.report_error(format!("Invalid empty input name: {function}",));
            "<invalid>".to_string()
        });
        let module_prefix = split;
        let function_name = NameId::new(self, module_prefix, name);

        let Some((_, entry)) = find_closest_match(self, function_name) else {
            tracing::debug!("no function found for {function}");
            return Ok(None);
        };
        let diagnostics: Vec<&Diagnostic> = find_closest_match::accumulated(self, function_name);
        handle_diagnostics(&diagnostics)?;

        let f = dwarf::resolve_function(self, entry);
        let diagnostics: Vec<&Diagnostic> = dwarf::resolve_function::accumulated(self, entry);
        handle_diagnostics(&diagnostics)?;
        Ok(f)
    }

    pub fn resolve_address_to_location(
        &self,
        address: u64,
    ) -> Result<Option<crate::ResolvedLocation>> {
        let address = Address::new(self, address);
        let loc = lookup_address(self, address);
        let Some(f) = lookup_closest_function(self, address) else {
            tracing::debug!("no function found for address {:#x}", address.address(self));
            return Ok(None);
        };
        let Some(function) = dwarf::resolve_function(self, f) else {
            tracing::debug!("failed to resolve function: {f:?}");
            return Ok(None);
        };
        let diagnostics: Vec<&Diagnostic> = dwarf::resolve_function::accumulated(self, f);
        handle_diagnostics(&diagnostics)?;
        tracing::debug!("returned function + loc: {f:?} / {loc:?}");
        Ok(loc.map(|loc| {
            let file = loc.file(self);
            crate::ResolvedLocation {
                function: function.name(self).to_string(),
                file: file.path(self).clone(),
                line: loc.line(self),
            }
        }))
    }

    pub fn resolve_position(
        &self,
        file: &str,
        line: u64,
        column: Option<u64>,
    ) -> Result<Option<crate::ResolvedAddress>> {
        let query = Position::new(self, file.to_string(), line, column);
        let pos = lookup_position(self, query);
        Ok(pos.map(|address| crate::ResolvedAddress { address }))
    }

    pub fn resolve_variables_at_address(
        &self,
        address: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<(
        Vec<crate::Variable>,
        Vec<crate::Variable>,
        Vec<crate::Variable>,
    )> {
        let address = Address::new(self, address);
        let loc = lookup_address(self, address);
        let f = lookup_closest_function(self, address);
        let diagnostics: Vec<&Diagnostic> = lookup_closest_function::accumulated(self, address);
        handle_diagnostics(&diagnostics)?;

        let Some(f) = f else {
            tracing::debug!("no function found for address {:#x}", address.address(self));
            return Ok(Default::default());
        };

        let vars = dwarf::resolve_function_variables(self, f);
        let diagnostics: Vec<&Diagnostic> = dwarf::resolve_function_variables::accumulated(self, f);
        handle_diagnostics(&diagnostics)?;

        let params = vars
            .params(self)
            .into_iter()
            .map(|param| output_variable(self, f, param, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        let locals = vars
            .locals(self)
            .into_iter()
            .filter(|var| {
                // for local variables, we want to make sure the variable
                // is defined before the current location
                if let Some(loc) = loc {
                    loc.line(self) > var.line(self)
                } else {
                    // if we don't have a location, we assume the param is valid
                    true
                }
            })
            .map(|var| output_variable(self, f, var, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        let globals = vars
            .globals(self)
            .into_iter()
            .map(|(var, address)| output_global_variable(self, var, address, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        Ok((params, locals, globals))
    }

    pub fn test_get_shape(&self) -> Result<Def<'_>> {
        let test_struct = test_get_def(self);
        let diagnostics: Vec<&Diagnostic> = test_get_def::accumulated(self);
        handle_diagnostics(&diagnostics)?;
        Ok(test_struct)
    }
}

fn output_variable<'db>(
    db: &'db dyn Db,
    f: FunctionIndexEntry<'db>,
    var: dwarf::Variable<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Variable> {
    let location = dwarf::resolve_data_location(db, f, var.origin(db), data_resolver)?;

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
            name: var.ty(db).display_name(db),
        }),
        value,
    })
}

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
            name: var.ty(db).display_name(db),
        }),
        value,
    })
}

#[salsa::db]
impl salsa::Database for DebugDatabaseImpl {
    fn salsa_event(&self, _event: &dyn Fn() -> salsa::Event) {
        // tracing already prints events, so nothing for us to do
    }
}

#[salsa::db]
impl Db for DebugDatabaseImpl {
    fn binary_file(&self) -> &File {
        self.binary_file.as_ref()
    }

    fn upcast(&self) -> &dyn Db {
        self
    }
}

#[salsa::tracked]
pub fn test_get_def(db: &dyn Db) -> Def<'_> {
    let index = index::index(db);

    // find the STATIC_TEST_STRUCT global constants
    let (_, static_test_struct) = index
        .data(db)
        .symbol_name_to_die
        .iter()
        .find(|(name, _)| {
            let name = name.name(db);
            name.contains("STATIC_TEST_STRUCT")
        })
        .expect("should find test struct");

    // get its DIE entry + type
    dwarf::resolve_type(db, static_test_struct.die(db)).expect("could not get type")
}
