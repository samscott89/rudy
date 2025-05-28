//! # DebugInfo DB
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

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::Arc,
};

use anyhow::{Context, Result};
use itertools::Itertools;
use object::{Object, ObjectSymbol};
use salsa::Accumulator;

mod dwarf;
mod file;
mod formatting;
#[cfg(test)]
mod tests;

pub use dwarf::Function;
pub use file::{File, FileId, SourceFile, load_relocatable_file};

use crate::data::Def;

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
    let _ = index(db);
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

    pub fn lookup_function(&self, function: &str) -> Result<Option<Function>> {
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
        // let diagnostics: Vec<&Diagnostic> = lookup_position::accumulated(self, query);
        // handle_diagnostics(&diagnostics)?;
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

#[salsa::interned]
pub struct NameId<'db> {
    #[return_ref]
    pub path: Vec<String>,
    #[return_ref]
    pub name: String,
}

impl<'db> NameId<'db> {
    pub fn as_path(&self, db: &'db dyn Db) -> String {
        let name = self.name(db);
        let path = self.path(db).iter().join("::");
        if path.is_empty() {
            name.to_string()
        } else {
            format!("{path}::{name}")
        }
    }
}

#[salsa::interned]
struct Symbol<'db> {
    #[return_ref]
    pub name_bytes: Vec<u8>,
}

#[salsa::tracked]
fn demangle<'db>(db: &'db dyn Db, sym: Symbol<'db>) -> NameId<'db> {
    let name_str = match std::str::from_utf8(sym.name_bytes(db).as_ref()) {
        Ok(name_str) => name_str,
        Err(_) => {
            tracing::warn!("Failed to demangle symbol: {:?}", sym.name_bytes(db));
            db.report_critical(format!(
                "Failed to parse symbol bytes to string: {:?}",
                sym.name_bytes(db)
            ));
            return NameId::new(db, vec![], "<invalid>".to_string());
        }
    };
    let demangled = rustc_demangle::demangle(name_str.as_ref());
    // return the demangled name as a string, without the trailing hash
    let demangled = format!("{demangled:#}");
    let mut split: Vec<String> = demangled.split("::").map(|s| s.to_owned()).collect();
    let name = split.pop().unwrap_or_else(|| {
        db.report_error(format!("Invalid empty symbol name: {demangled}",));
        "<invalid>".to_string()
    });
    NameId::new(db, split, name)
}

#[salsa::tracked]
pub struct FunctionIndexEntry<'db> {
    die: dwarf::DieEntryId<'db>,
}

#[salsa::tracked]
pub struct SymbolIndexEntry<'db> {
    address: u64,
    die: dwarf::DieEntryId<'db>,
}

#[salsa::tracked]
pub struct TypeIndexEntry<'db> {
    die: dwarf::DieEntryId<'db>,
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(return_ref)]
pub fn index<'db>(db: &'db dyn Db) -> dwarf::Index<'db> {
    let binary_file = db.binary_file();
    let Some(object) = binary_file.object() else {
        return dwarf::Index::new(db, Default::default());
    };

    // initialize structs
    let mut function_name_to_die: BTreeMap<NameId<'_>, FunctionIndexEntry<'_>> = Default::default();
    let mut symbol_name_to_die: BTreeMap<NameId<'_>, SymbolIndexEntry<'_>> = Default::default();
    let mut type_name_to_die: BTreeMap<NameId<'_>, TypeIndexEntry<'_>> = Default::default();
    let mut die_to_type_name: BTreeMap<dwarf::DieEntryId<'_>, NameId<'_>> = Default::default();
    let mut cu_to_base_addr: BTreeMap<dwarf::CompilationUnitId<'_>, u64> = Default::default();
    let mut address_range_to_cu: Vec<(u64, u64, dwarf::CompilationUnitId<'_>)> = Default::default();
    let mut address_range_to_function: Vec<(u64, u64, NameId<'_>)> = Default::default();
    let mut file_to_cu: BTreeMap<SourceFile<'_>, Vec<dwarf::CompilationUnitId<'_>>> =
        Default::default();

    let mut names_by_file: HashMap<FileId<'db>, BTreeMap<Vec<u8>, _>> = HashMap::new();

    let object_map = object.object_map();
    for s in object_map.symbols() {
        let symbol = Symbol::new(db, s.name());
        let demangled_name = demangle(db, symbol);
        let file = s.object(&object_map).path();
        let file = match std::str::from_utf8(file) {
            Ok(p) => p,
            Err(e) => {
                db.report_critical(format!("Failed to parse object file path: {file:?}: {e}"));
                continue;
            }
        };
        let member = s
            .object(&object_map)
            .member()
            .and_then(|m| match std::str::from_utf8(m) {
                Ok(m) => Some(m.to_string()),
                Err(e) => {
                    db.report_critical(format!("Failed to parse object file member: {m:?}: {e}"));
                    None
                }
            });
        let file = FileId::new(db, file.to_string(), member, true);
        names_by_file.entry(file).or_default().insert(
            // trim the leading `_` character that macos adds when using STAB entries
            s.name()[1..].to_vec(),
            (s.address(), demangled_name),
        );
    }

    // append names from the root binary (if it has any)
    for symbol in object.symbols() {
        let name = symbol.name_bytes().unwrap();
        if name.is_empty() {
            tracing::debug!("empty symbol: {symbol:#?}");
            continue;
        }
        let symbol_name = Symbol::new(db, name);
        let demangled_name = demangle(db, symbol_name);

        let file = binary_file.file_id(db);
        names_by_file
            .entry(file)
            .or_default()
            .insert(name.to_vec(), (symbol.address(), demangled_name));
    }

    for (file_id, names) in names_by_file {
        // let path = file_id.path(db);
        // // TODO(Sam): we can use `object::ArchiveFile::parse` to parse this and extract
        // // the individual `.o` files within
        // if path.ends_with("rlib") {
        //     let names = names.iter().map(|(_, (_, n))| n.as_path(db)).join(", ");
        //     tracing::trace!("skipping .rlib file: {path} with referenced symbols: {names}");
        //     continue;
        // }

        let file_entries = dwarf::index_symbols(db, file_id, names);

        function_name_to_die.extend(file_entries.function_name_to_die);
        symbol_name_to_die.extend(file_entries.symbol_name_to_die);
        address_range_to_function.extend(file_entries.address_range_to_function);
        cu_to_base_addr.extend(file_entries.cu_to_base_addr);

        let (name_to_die, die_to_name) = dwarf::index_types(db, file_id);
        type_name_to_die.extend(name_to_die);
        die_to_type_name.extend(die_to_name);

        let roots = dwarf::parse_roots(db, file_id);
        for root in roots {
            let cu = root.cu(db);
            if let Some(base_addr) = cu_to_base_addr.get(&root.cu(db)) {
                let (start, end) = root.address_range(db);
                address_range_to_cu.push((base_addr + start, base_addr + end, cu));
            }

            for file in root.files(db) {
                file_to_cu.entry(*file).or_default().push(cu)
            }
        }
    }

    // sort the lists
    address_range_to_function.sort_unstable();
    address_range_to_cu.sort_unstable();

    dwarf::Index::new(
        db,
        dwarf::IndexData {
            function_name_to_die,
            symbol_name_to_die,
            type_name_to_die,
            die_to_type_name,
            cu_to_base_addr,
            address_range_to_cu,
            address_range_to_function,
            file_to_cu,
        },
    )
}

#[salsa::tracked]
fn find_closest_match<'db>(
    db: &'db dyn Db,
    function_name: NameId<'db>,
) -> Option<(NameId<'db>, FunctionIndexEntry<'db>)> {
    // check if exact name exists in index
    let index = index(db);
    if let Some(entry) = index.data(db).function_name_to_die.get(&function_name) {
        return Some((function_name, *entry));
    }

    // otherwise, find the closest match by scanning the index
    let name = function_name.name(db);
    let module_prefix = function_name.path(db);

    index
        .data(db)
        .function_name_to_die
        .iter()
        .find_map(|(indexed_name, entry)| {
            if indexed_name.name(db) == name
                && indexed_name.path(db).ends_with(module_prefix.as_slice())
            {
                Some((*indexed_name, *entry))
            } else {
                None
            }
        })
}

#[salsa::interned]
struct Position<'db> {
    pub file: String,
    pub line: u64,
    pub column: Option<u64>,
}

#[salsa::tracked]
fn lookup_position<'db>(db: &'db dyn Db, query: Position<'db>) -> Option<u64> {
    let file_name = query.file(db);
    let file = SourceFile::new(db, file_name);

    // find compilation units that cover the provided file
    let index = index(db);
    let Some(cu_ids) = index.data(db).file_to_cu.get(&file) else {
        tracing::debug!(
            "no compilation units found for file: {} in index {:#?}",
            file.path(db),
            index.data(db).file_to_cu
        );
        return None;
    };

    if cu_ids.is_empty() {
        tracing::debug!("No compilation units found for file: {}", file.path(db));
        return None;
    }

    let mut closest_match: Option<u64> = None;
    let mut closest_line = u64::MAX;

    // find closest match to this line + column within the files
    for cu in cu_ids {
        let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu).copied() else {
            tracing::debug!("no base address found for {cu:?}");
            continue;
        };
        tracing::debug!("looking for matches in {cu:?}");
        if let Some((addr, distance)) = dwarf::location_to_address(db, *cu, query.clone()) {
            tracing::debug!("found match  {addr:#x} at distance {distance}");
            if distance < closest_line {
                tracing::debug!(
                    "base: {base_addr:#x} + addr: {addr:#x} = {:#x}",
                    base_addr + addr
                );
                closest_match = Some(base_addr + addr);
                closest_line = distance;
            }
            if distance == 0 {
                // we found an exact match, so we can stop
                break;
            }
        }
    }

    // return the address
    closest_match
}

#[tracing::instrument(skip(db))]
#[salsa::tracked]
fn lookup_address<'db>(
    db: &'db dyn Db,
    address: Address<'db>,
) -> Option<dwarf::ResolvedLocation<'db>> {
    let address = address.address(db);
    let index = index(db);
    let cu_index = &index.data(db).address_range_to_cu;
    let range_start = cu_index.partition_point(|(start, _, _)| *start < address);
    let range = &cu_index[..range_start];
    if range.is_empty() {
        return None;
    }
    tracing::trace!("found {} CU in range", range.len());
    // iterate through the matching ranges in _reverse_ order
    // since we know all of the ranges match the address on the left
    // hand side of the range
    for (start, end, cu) in range.iter().rev() {
        if address > *end {
            tracing::trace!("{address:#x} after {end:#x} for {cu:?}");
            continue;
        };
        tracing::debug!("found CU: {cu:?} ({start:#x}, {end:#x})");

        let Some(base_addr) = index.data(db).cu_to_base_addr.get(cu) else {
            tracing::trace!("no base address found for {cu:?}");
            continue;
        };

        let Some(relative_addr) = address.checked_sub(*base_addr) else {
            tracing::trace!("address {address:#x} is before base address {base_addr:#x}");
            continue;
        };

        if let Some(loc) = dwarf::address_to_location(db, *cu, relative_addr) {
            tracing::trace!("resolved location: {loc:#?}");
            return Some(loc);
        } else {
            tracing::debug!("location not found");
        }
    }
    None
}

#[salsa::interned]
struct Address<'db> {
    address: u64,
}

#[salsa::tracked]
fn lookup_closest_function<'db>(
    db: &'db dyn Db,
    address: Address<'db>,
) -> Option<FunctionIndexEntry<'db>> {
    let address = address.address(db);
    tracing::debug!("looking up function for address {address:#x}");
    let index = index(db);
    let function_index = &index.data(db).address_range_to_function;

    let range_start = function_index.partition_point(|(start, _, _)| *start < address);
    let range = &function_index[..range_start];
    if range.is_empty() {
        tracing::warn!(
            "no function found in range: index: {function_index}\naddress: {address}\nrange: {range}",
            function_index = function_index
                .iter()
                .map(|(start, end, name)| format!("{start:#x}..{end:#x} ({})", name.as_path(db)))
                .join(",\n\t"),
            range = range
                .iter()
                .map(|(start, end, name)| format!("{start:#x}..{end:#x} ({})", name.as_path(db)))
                .join(",\n\t")
        );
        return None;
    }
    tracing::debug!("found {} functions in range", range.len());
    for (start, end, name) in range {
        if address < *start {
            tracing::error!("address {address:#x} is before start {start:#x}");
        }
        if address > *end {
            tracing::trace!("address {address:#x} is after end {end:#x}");
            continue;
        };
        tracing::trace!("function: {name:?} ({start:#x}, {end:#x})");
        let function = index.data(db).function_name_to_die.get(name)?;
        let die_id = function.die(db);
        let base_address = index.data(db).cu_to_base_addr.get(&die_id.cu(db))?;
        // do the more precise check using the exact ranges iterator
        // for the entry
        let relative_address = address - base_address;
        // check the precise address is in the ranges covered
        // by the function (in the case of non-contiguous ranges)
        if dwarf::address_in_entry(db, relative_address, die_id) {
            return Some(*function);
        } else {
            tracing::debug!("not in entry");
        }
    }
    None
}

#[salsa::tracked]
pub fn test_get_def(db: &dyn Db) -> Def<'_> {
    let index = index(db);

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
    static_test_struct
        .die(db)
        .ty(db)
        .expect("could not get type")
}

pub(crate) fn get_def<'db>(db: &'db dyn Db, name: NameId<'db>) -> Result<Option<Def<'db>>> {
    // get the DIE for the name
    let index = index(db);
    let Some(entry) = index.data(db).type_name_to_die.get(&name) else {
        tracing::warn!(
            "could not find type {} in index: {:#?}",
            name.as_path(db),
            index.data(db).type_name_to_die
        );
        return Ok(None);
    };

    // get the type
    let ty = dwarf::resolve_type_offset(db, entry.die(db));

    // resolve diagnostics
    let diagnostics: Vec<&Diagnostic> = dwarf::resolve_type_offset::accumulated(db, entry.die(db));
    handle_diagnostics(&diagnostics)?;

    Ok(ty)
}
