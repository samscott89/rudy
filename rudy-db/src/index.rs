//! Index building for fast debug info lookups

use std::collections::BTreeMap;

use anyhow::Context;
use itertools::Itertools;
use rudy_types::TypeLayout;

use crate::address_tree::FunctionAddressInfo;
use crate::database::Db;
use crate::dwarf::{self, FunctionIndexEntry, SymbolName, TypeName};
use crate::file::{Binary, DebugFile, SourceFile};
use crate::index::symbols::{DebugFiles, SymbolIndex};

pub mod symbols;

#[salsa::tracked(debug)]
pub struct Index<'db> {
    pub debug_files: DebugFiles,
    pub symbol_index: SymbolIndex,
    pub indexed_debug_files: Vec<DebugFile>,
    pub source_to_file: BTreeMap<SourceFile<'db>, Vec<DebugFile>>,
}

impl<'db> Index<'db> {
    pub fn get_function(
        &self,
        db: &'db dyn Db,
        name: &SymbolName,
    ) -> Option<(DebugFile, FunctionIndexEntry)> {
        let sym = self
            .symbol_index(db)
            .get_functions_by_lookup_name(&name.lookup_name)?
            .get(name)
            .or_else(|| {
                tracing::info!("{name} not found in root symbol index");
                None
            })?
            .clone();
        let indexed = dwarf::index_debug_file_full(db, sym.debug_file).data(db);
        indexed
            .functions
            .get(name)
            .cloned()
            .or_else(|| {
                tracing::debug!(
                    "Function {name} not found in debug file {}",
                    sym.debug_file.name(db)
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
#[salsa::tracked(returns(ref))]
pub fn debug_index<'db>(db: &'db dyn Db, binary: Binary) -> Index<'db> {
    let Ok((debug_files, symbol_index)) = symbols::index_symbol_map(db, binary)
        .with_context(|| {
            format!(
                "Failed to index debug files for binary: {}",
                binary.file(db).path(db)
            )
        })
        .inspect_err(|e| {
            db.report_critical(format!("Failed to index debug files: {e}"));
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

    let binary_path = binary
        .file(db)
        .path(db)
        .replace("/deps/", "/")
        .replace("-", "_");

    // TODO: this logic is not super robust. Perhaps we could instead
    // scan through all CUs to find source files that match the binary path?
    let indexed_debug_files = debug_files
        .iter()
        .filter_map(|((file, _member), debug_file)| {
            // filter out files that are not related to the binary
            if file
                .replace("/deps/", "/")
                .replace("-", "_")
                .contains(&binary_path)
            {
                Some(*debug_file)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut source_file_index: BTreeMap<SourceFile<'_>, Vec<DebugFile>> = Default::default();

    for debug_file in &indexed_debug_files {
        let sources = dwarf::index_debug_file_sources(db, *debug_file);
        for source in sources {
            source_file_index
                .entry(*source)
                .or_default()
                .push(*debug_file);
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
pub fn find_all_by_address(
    db: &dyn Db,
    binary: Binary,
    address: u64,
) -> Vec<(u64, &FunctionAddressInfo)> {
    // first, find the closest function by address
    let index = debug_index(db, binary).symbol_index(db);
    let Some((_, function_symbols)) = index.function_at_address(address) else {
        return vec![];
    };

    // then, query the debug file index to find the actual matching addresses
    function_symbols
        .iter()
        .flat_map(|s| {
            // our index says that this symbol is _approximately_ at `address`
            // but we want to find the exact address in the debug file

            // we also need to adjust the address by the symbol's base address

            let debug_file = s.debug_file;
            let indexed = dwarf::index_debug_file_full(db, debug_file).data(db);
            let Some(f) = indexed.functions.get(&s.name) else {
                tracing::warn!(
                    "No function found for symbol {} in debug file {}",
                    s.name,
                    debug_file.name(db)
                );
                return vec![];
            };
            let f = f.data(db);
            let Some((relative_start, _)) = f.relative_address_range else {
                return vec![]
            };

            // compute the necessary address slide
            let slide = s.address - relative_start;

            debug_assert!(
                debug_file.relocatable(db) || slide == 0,
                "Expected slide to be 0 for non-relocatable debug files, got relocatable={} and slide={slide}",
                debug_file.relocatable(db),
            );

            let relative_address = address - slide;

            tracing::info!(
                "Resolving function {} at address {address:#x} with slide {slide:#x} (relative addr: {relative_address:#x}) in debug file {}",
                s.name,
                debug_file.name(db)
            );

            // if not relocatable, we can use the address directly
            indexed.function_addresses.query_address(relative_address).into_iter().map(|f| {
                // return the relative address and the function info
                (relative_address, f)
            }).collect_vec()
        })
        .collect()
}

/// Resolve a type by name in the debug information
pub fn resolve_type(
    db: &dyn Db,
    binary: Binary,
    type_name: &str,
) -> anyhow::Result<Option<TypeLayout>> {
    let parsed = TypeName::parse(&[], type_name)?;
    tracing::info!("Finding type '{parsed}'");
    let index = debug_index(db, binary);

    let indexed_debug_files = index.indexed_debug_files(db);

    if indexed_debug_files.is_empty() {
        tracing::warn!(
            "No indexed debug files found for binary: {}",
            binary.file(db).path(db)
        );
        return Ok(None);
    }

    // Search through all debug files to find the type
    for debug_file in indexed_debug_files {
        let indexed = dwarf::index_debug_file_full(db, debug_file).data(db);

        // Look for types that match the given name
        let entries = indexed
            .types
            .iter()
            .filter(|(name, _)| {
                if name.name.starts_with(&parsed.name) {
                    tracing::info!("Found type name: {} vs {}", name.name, parsed.name);
                    tracing::info!("Checking type '{name}' vs '{parsed}'");
                    if !name.typedef.matching_type(&parsed.typedef) {
                        tracing::info!(
                            "Type '{:#?}' does not match '{:#?}'",
                            name.typedef,
                            parsed.typedef
                        );
                        return false;
                    }
                    name.module.segments.ends_with(&parsed.module.segments)
                } else {
                    false
                }
            })
            .flat_map(|(_, entry)| entry);

        for entry in entries {
            // Resolve the type using the DWARF resolution logic
            match crate::dwarf::resolve_type_offset(db, entry.die(db)) {
                Ok(typedef) => {
                    // Successfully resolved the type
                    tracing::info!(
                        "Resolved type '{type_name}' to {typedef:#?} in {}",
                        entry.die(db).print(db)
                    );
                    // nowe we've found a match, we can fully resolve the type
                    return Ok(Some(crate::dwarf::fully_resolve_type(
                        db, debug_file, &typedef,
                    )?));
                }
                Err(e) => {
                    tracing::warn!("Failed to resolve type '{type_name}': {e:?}");
                }
            }
        }
        tracing::trace!(
            "No type '{}' found in debug file: {}",
            type_name,
            debug_file.name(db)
        );
    }

    // Type not found in any debug file
    Ok(None)
}
