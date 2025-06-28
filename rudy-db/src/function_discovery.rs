use anyhow::Result;
use rudy_types::TypeLayout;
use std::collections::BTreeMap;

use crate::{
    DiscoveredMethod,
    database::Db,
    dwarf::{FunctionSignature, resolve_function_signature},
    file::Binary,
};

/// Result of analyzing a symbol for method discovery
#[derive(Debug, Clone)]
pub enum SymbolAnalysisResult<'db> {
    /// Successfully discovered a method
    DiscoveredFunction {
        signature: FunctionSignature<'db>,
    },
    /// Symbol exists but is not a method
    NotAMethod {
        reason: String,
    },
    /// Symbol has debug info but variable resolution failed
    VariableResolutionError {
        error: String,
    },
    /// Symbol has debug info but method analysis failed
    AnalysisError {
        error: String,
    },
    /// Symbol exists but has no debug information
    NoDebugInfo,
    TypeResolutionError {
        error: String,
    },
    UnindexedFile {
        file: String,
    },
}

/// Discover all methods in the binary and organize them by symbol name
///
/// This returns debugging information about each symbol, including whether it was successfully analyzed,
/// if it was not a method, or if there was an error during analysis.
pub fn discover_all_functions_debug(
    db: &dyn Db,
    binary: Binary,
) -> Result<BTreeMap<String, SymbolAnalysisResult>> {
    let index = crate::index::debug_index(db, binary);

    // get the debug files that we eagerly loaded
    // we'll eagerly scan _all_ functions in this file
    // while only selectively scanning methods elsewhere
    // this is mostly important on macos platforms where we
    // the debug info points to many files, like the rustc .rlib
    // files containing more debug symbols.
    let indexed_debug_files = index.indexed_debug_files(db);

    let symbol_index = index.symbol_index(db);
    let mut symbol_results = BTreeMap::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        tracing::debug!("Processing symbol: {}", symbol.name);

        let symbol_name = symbol.name.to_string();

        let debug_file = symbol.debug_file;

        // skip symbols in external debug files that
        // we don't want to load
        if !indexed_debug_files.contains(&debug_file) {
            symbol_results.insert(
                symbol_name.clone(),
                SymbolAnalysisResult::UnindexedFile {
                    file: debug_file.file(db).path(db).to_string(),
                },
            );
            continue;
        }

        // finally, get the function index entry
        let Some(function_entry) = crate::dwarf::index_debug_file_full(db, debug_file)
            .data(db)
            .functions
            .get(&symbol.name)
        else {
            symbol_results.insert(symbol_name.clone(), SymbolAnalysisResult::NoDebugInfo);
            continue;
        };

        match resolve_function_signature(db, *function_entry) {
            Ok(sig) => {
                symbol_results.insert(
                    symbol_name,
                    SymbolAnalysisResult::DiscoveredFunction { signature: sig },
                );
            }
            Err(e) => {
                symbol_results.insert(
                    symbol_name.clone(),
                    SymbolAnalysisResult::AnalysisError {
                        error: format!("{e:?}"),
                    },
                );
            }
        }
    }

    Ok(symbol_results)
}

/// Discover all methods in the binary and organize them by type of the first
pub fn discover_all_methods(
    db: &dyn Db,
    binary: Binary,
) -> Result<BTreeMap<String, Vec<DiscoveredMethod>>> {
    let index = crate::index::debug_index(db, binary);

    // get the debug files that we eagerly loaded
    // we'll eagerly scan _all_ functions in this file
    // while only selectively scanning methods elsewhere
    // this is mostly important on macos platforms where we
    // the debug info points to many files, like the rustc .rlib
    // files containing more debug symbols.
    let indexed_debug_files = index.indexed_debug_files(db);

    let symbol_index = index.symbol_index(db);
    let mut methods_by_type: BTreeMap<String, Vec<DiscoveredMethod>> = BTreeMap::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        tracing::debug!("Processing symbol: {}", symbol.name);

        let debug_file = symbol.debug_file;

        // skip symbols in external debug files that
        // we don't want to load
        if !indexed_debug_files.contains(&debug_file) {
            continue;
        }

        // finally, get the function index entry
        let Some(function_entry) = crate::dwarf::index_debug_file_full(db, debug_file)
            .data(db)
            .functions
            .get(&symbol.name)
        else {
            continue;
        };

        match resolve_function_signature(db, *function_entry) {
            Ok(sig) => {
                let params = sig.params(db);
                let Some(first_param) = params.first() else {
                    // no parameters, skip
                    continue;
                };

                let Some(self_type) = sig.self_type(db) else {
                    // Not a method, skip
                    continue;
                };

                let first_type = first_param.ty(db).dereferenced();

                if matches!(first_type, TypeLayout::Alias(a) if a.name == "unknown") {
                    // Skip methods with an unknown type
                    continue;
                }

                methods_by_type
                    .entry(first_type.display_name())
                    .or_default()
                    .push(DiscoveredMethod {
                        name: sig.name(db).to_string(),
                        full_name: symbol.name.to_string(),
                        signature: sig.print_sig(db),
                        address: symbol.address,
                        self_type,
                        // defaults to callable
                        callable: true,
                    });
            }
            Err(e) => {
                tracing::trace!(
                    "Failed to resolve function signature for {}: {e}",
                    symbol.name
                );
            }
        }
    }

    Ok(methods_by_type)
}

/// Discover all methods in the binary and organize them by type of the first
pub fn discover_all_methods_for_type(
    db: &dyn Db,
    binary: Binary,
    target_type: &TypeLayout,
) -> Result<Vec<DiscoveredMethod>> {
    let index = crate::index::debug_index(db, binary);

    // get the debug files that we eagerly loaded
    // we'll eagerly scan _all_ functions in this file
    // while only selectively scanning methods elsewhere
    // this is mostly important on macos platforms where we
    // the debug info points to many files, like the rustc .rlib
    // files containing more debug symbols.
    let indexed_debug_files = index.indexed_debug_files(db);

    let symbol_index = index.symbol_index(db);
    let mut methods: Vec<DiscoveredMethod> = Vec::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        tracing::debug!("Processing symbol: {}", symbol.name);

        let debug_file = symbol.debug_file;

        let symbol_name = symbol.name.to_string();

        // skip symbols in external debug files that
        // we don't want to load
        if !indexed_debug_files.contains(&debug_file) {
            continue;
        }

        // finally, get the function index entry
        let Some(function_entry) = crate::dwarf::index_debug_file_full(db, debug_file)
            .data(db)
            .functions
            .get(&symbol.name)
        else {
            tracing::debug!("No function entry found for symbol: {symbol_name}",);
            continue;
        };

        match resolve_function_signature(db, *function_entry) {
            Ok(sig) => {
                let params = sig.params(db);
                let Some(first_param) = params.first() else {
                    // no parameters, skip
                    continue;
                };

                if !first_param.ty(db).dereferenced().matching_type(target_type) {
                    // Not the target type, skip
                    continue;
                }

                let Some(self_type) = sig.self_type(db) else {
                    // Not a method, skip
                    continue;
                };

                methods.push(DiscoveredMethod {
                    name: sig.name(db).to_string(),
                    full_name: symbol_name,
                    signature: sig.print_sig(db),
                    address: symbol.address,
                    self_type,
                    // defaults to callable
                    callable: true,
                });
            }
            Err(e) => {
                tracing::trace!(
                    "Failed to resolve function signature for {}: {}",
                    symbol_name,
                    e
                );
            }
        }
    }

    Ok(methods)
}
