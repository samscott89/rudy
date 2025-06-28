use anyhow::Result;
use std::collections::BTreeMap;

use crate::{
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

/// Discover all methods in the binary and organize them by type
///
/// This provides a comprehensive view of all available methods across all types.
///
/// # Returns
///
/// A map from type names to their discovered methods
/// Debug version of method discovery that captures all symbol processing results
pub fn discover_all_methods_debug(
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

        let Some((debug_file, function_entry)) = index.get_function(db, &symbol.name) else {
            symbol_results.insert(symbol_name, SymbolAnalysisResult::NoDebugInfo);
            continue;
        };

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

        match resolve_function_signature(db, function_entry) {
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
