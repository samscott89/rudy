//! Index building for fast debug info lookups

use std::collections::BTreeMap;

use anyhow::Context;
use itertools::Itertools;
use rudy_types::TypeLayout;

use crate::database::Db;
use rudy_dwarf::address_tree::FunctionAddressInfo;
use rudy_dwarf::symbols::{DebugFiles, SymbolIndex};
use rudy_dwarf::{self, CompilationUnitId, FunctionIndexEntry, SymbolName, TypeName};
use rudy_dwarf::{Binary, DebugFile, SourceFile};

#[salsa::tracked(debug)]
pub struct Index<'db> {
    #[returns(ref)]
    pub debug_files: DebugFiles,
    #[returns(ref)]
    pub symbol_index: SymbolIndex,
    pub indexed_debug_files: Vec<DebugFile>,
    #[returns(ref)]
    pub source_to_file: BTreeMap<SourceFile<'db>, Vec<DebugFile>>,
}

impl<'db> Index<'db> {
    #[tracing::instrument(skip_all)]
    pub fn get_function(
        &self,
        db: &'db dyn Db,
        name: &SymbolName,
    ) -> Option<(DebugFile, FunctionIndexEntry)> {
        let symbol_index = self.symbol_index(db);
        let sym = symbol_index
            .get_functions_by_lookup_name(&name.lookup_name)?
            .get(name)
            .or_else(|| {
                tracing::info!("{name} not found in root symbol index");
                None
            })?
            .clone();
        let indexed = symbol_index.function_index(db, sym.debug_file)?;
        indexed
            .by_symbol_name(db)
            .get(name)
            .cloned()
            .or_else(|| {
                tracing::debug!(
                    "Function {name} not found in debug file {}, {:#?}",
                    sym.debug_file.name(db),
                    indexed.by_symbol_name(db)
                );
                None
            })
            .map(|entry| (sym.debug_file, entry))
    }
    // #[allow(dead_code)]
    // pub fn lookup_symbol(
    //     &self,
    //     db: &'db dyn Db,
    //     name: NameId<'db>,
    // ) -> Option<SymbolIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.symbols.get(&name).cloned()
    // }
    // #[allow(dead_code)]
    // pub fn lookup_module(
    //     &self,
    //     db: &'db dyn Db,
    //     name: NameId<'db>,
    // ) -> Option<ModuleIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.modules.get(&name).cloned()
    // }
    // #[allow(dead_code)]
    // pub fn lookup_type(&self, db: &'db dyn Db, name: NameId<'db>) -> Option<TypeIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.types.get(&name).cloned()
    // }
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[tracing::instrument(skip_all)]
#[salsa::tracked(returns(ref))]
pub fn debug_index<'db>(db: &'db dyn Db, binary: Binary) -> Index<'db> {
    let Ok((debug_files, symbol_index)) = rudy_dwarf::symbols::index_symbol_map(db, binary)
        .with_context(|| {
            format!(
                "Failed to index debug files for binary: {}",
                binary.name(db)
            )
        })
        .inspect_err(|e| {
            db.report_critical(format!("Failed to index debug files: {e:?}"));
        })
    else {
        return Index::new(
            db,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
    };

    tracing::trace!("Debug files found: {debug_files:#?}",);

    let mut indexed_debug_files = vec![];

    let mut source_file_index: BTreeMap<SourceFile<'_>, Vec<DebugFile>> = Default::default();

    // attempt to detect the current cargo workspace

    let workspace_root = rudy_dwarf::file::detect_cargo_root();
    if workspace_root.is_none() {
        tracing::warn!(
            "Could not find Cargo workspace root, debug and source file indexing may be incomplete."
        );
    }

    for debug_file in debug_files.values() {
        let (_, sources) = rudy_dwarf::index_debug_file_sources(db, *debug_file);
        for source in sources {
            source_file_index
                .entry(*source)
                .or_default()
                .push(*debug_file);
        }
        if let Some(ref workspace_root) = workspace_root {
            // if the source file is external, we don't want to index it
            // as it may not be available in the workspace
            if sources.iter().any(|s| {
                let p = s.path(db);
                p.starts_with(workspace_root) || p.starts_with(".")
            }) {
                tracing::debug!(
                    "Indexing debug file {} with local sources.",
                    debug_file.name(db),
                );
                // add to the indexed debug files list
                indexed_debug_files.push(*debug_file);
            } else {
                tracing::debug!(
                    "Skipping debug file {} with no local sources.",
                    debug_file.name(db),
                );
            }
        }
    }

    Index::new(
        db,
        debug_files,
        symbol_index,
        indexed_debug_files,
        source_file_index,
    )
}

#[salsa::tracked]
pub fn find_closest_function<'db>(
    db: &'db dyn Db,
    binary: Binary,
    function_name: &'db str,
) -> Option<(SymbolName, DebugFile)> {
    let index = debug_index(db, binary);
    let mut segments = function_name
        .split("::")
        .map(|s| s.to_owned())
        .collect::<Vec<String>>();
    let name = segments.pop()?;
    let module: Vec<String> = segments.iter().map(|s| s.to_string()).collect();

    for (indexed_name, entry) in index.symbol_index(db).get_functions_by_lookup_name(&name)? {
        // check if the name matches and the path ends with the module prefix
        if indexed_name.matches_name_and_module(&name, &module) {
            return Some((indexed_name.clone(), entry.debug_file));
        }
    }
    None
}

/// Finds all functions
pub fn find_all_by_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: u64,
) -> Vec<(u64, CompilationUnitId<'db>, &'db FunctionAddressInfo)> {
    // first, find the closest function by address
    let index = debug_index(db, binary).symbol_index(db);
    let Some((_, function_symbols)) = index.function_at_address(address) else {
        tracing::debug!(
            "No function found at address {address:#x} in binary {}",
            binary.name(db)
        );
        return vec![];
    };

    // then, query the debug file index to find the actual matching addresses
    function_symbols
        .iter()
        .flat_map(|s| {
            let debug_file = s.debug_file;
            // our index says that this symbol is _approximately_ at `address`
            // but we want to find the exact address in the debug file

            // we also need to adjust the address by the symbol's base address

            let Some(function_index) = index.function_index(db, debug_file) else {
                tracing::warn!(
                    "No function index found for debug file {}",
                    debug_file.name(db)
                );
                return vec![];
            };

            function_index
                .by_absolute_address(db)
                .query_address(address)
                .into_iter()
                .filter_map(|f| {
                    // the relative address
                    // in the case of no relocations, f.start == s.address
                    // and this returns just `address`
                    let relative_address = f.relative_start +  (address - s.address);
                    let function = function_index.by_symbol_name(db).get(&f.name)?.data(db);
                    tracing::info!(
                        "Found function {f:#?} for address {address:#x} and symbol {s:#?} at address {relative_address:#x} in debug file {}",
                        debug_file.name(db)
                    );
                    let cu: CompilationUnitId<'_> = function.declaration_die.cu(db);
                    Some((relative_address, cu, f))
                })
                .collect_vec()
        })
        .collect()
}

/// Resolve a type by name in the debug information
pub fn resolve_type(
    db: &dyn Db,
    binary: Binary,
    type_name: &str,
) -> anyhow::Result<Option<(TypeLayout, DebugFile)>> {
    let (segments, name) = if let Some((name, generics)) = type_name.split_once('<') {
        // If the type name has generics, we need to handle them separately
        let mut segments = name
            .split("::")
            .map(|s| s.to_owned())
            .collect::<Vec<String>>();
        let type_name = segments.pop().context("Type name cannot be empty")?;

        (segments, format!("{type_name}<{generics}"))
    } else {
        let mut segments = type_name
            .split("::")
            .map(|s| s.to_owned())
            .collect::<Vec<String>>();
        let type_name = segments.pop().context("Type name cannot be empty")?;
        (segments, type_name)
    };

    let parsed = TypeName::parse(&segments, &name)?;
    tracing::info!("Finding type '{parsed}'");
    let index = debug_index(db, binary);

    let indexed_debug_files = index.indexed_debug_files(db);

    if indexed_debug_files.is_empty() {
        tracing::warn!(
            "No indexed debug files found for binary: {}",
            binary.name(db)
        );
        return Ok(None);
    }

    // Search through all debug files to find the type
    for debug_file in indexed_debug_files {
        let Some(type_def) = rudy_dwarf::find_type_by_name(db, debug_file, parsed.clone()) else {
            continue;
        };

        return Ok(Some((
            rudy_dwarf::resolve_type_offset(db, type_def)?,
            debug_file,
        )));
    }

    // Type not found in any debug file
    Ok(None)
}
