use std::collections::BTreeMap;

use anyhow::Result;
use rudy_dwarf::{
    Binary,
    function::{FunctionSignature, resolve_function_signature},
    types::DieTypeDefinition,
};
use rudy_types::Layout;

use crate::{DiscoveredMethod, database::Db};

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
                    file: debug_file.name(db),
                },
            );
            continue;
        }

        // finally, get the function index entry
        let Some(function_index) = symbol_index.function_index(db, debug_file) else {
            continue;
        };
        let Some(function_entry) = function_index.by_symbol_name(db).get(&symbol.name) else {
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

        // Use targeted function indexing instead of full indexing
        let Some(function_index) = symbol_index.function_index(db, debug_file) else {
            continue;
        };
        let Some(function_entry) = function_index.by_symbol_name(db).get(&symbol.name) else {
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

                let first_type = first_param.ty(db).layout.dereferenced();

                if matches!(first_type, Layout::Alias { name } if name == "unknown") {
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

/// Optimized method discovery using DWARF structure knowledge
///
/// This implements the optimization plan documented in notes/method-discovery-optimization.md:
/// 1. Direct method discovery from type DIE structure (methods nested under structs)
/// 2. Trait implementation discovery using module traversal ({impl#N} patterns)
/// 3. Fallback to symbol-based search for edge cases
pub fn discover_methods_for_type(
    db: &dyn Db,
    binary: Binary,
    target_type: &DieTypeDefinition,
) -> Result<Vec<DiscoveredMethod>> {
    let mut methods = Vec::new();

    // Phase 1: Direct method discovery - look for methods structurally nested under the type DIE
    if let Some(direct_methods) = discover_direct_methods(db, target_type)? {
        methods.extend(direct_methods);
        tracing::debug!(
            "Found {} direct methods for type {}",
            methods.len(),
            target_type.display_name()
        );
    }

    // Phase 2: Trait implementation discovery - search for {impl#N} blocks in the same module
    let trait_methods = discover_trait_impl_methods(db, binary, target_type)?;
    methods.extend(trait_methods);
    tracing::debug!(
        "Found {} total methods after trait discovery for type {}",
        methods.len(),
        target_type.display_name()
    );

    tracing::debug!(
        "Final method count for type {}: {}",
        target_type.display_name(),
        methods.len()
    );
    Ok(methods)
}

/// Phase 1: Discover methods that are structurally nested under the type DIE
///
/// In DWARF, Rust methods are often represented as functions nested directly under the type DIE.
fn discover_direct_methods(
    db: &dyn rudy_dwarf::DwarfDb,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
) -> Result<Option<Vec<DiscoveredMethod>>> {
    use rudy_dwarf::parser::Parser;
    use rudy_dwarf::parser::functions::child_functions_parser;

    let type_die = target_type.location;

    // Use the function parser to find all functions nested under this type DIE
    let functions = child_functions_parser().parse(db, type_die)?;

    if functions.is_empty() {
        tracing::debug!(
            "No direct methods found for type {}",
            target_type.display_name()
        );
        return Ok(None);
    }

    let functions_count = functions.len();
    let mut methods = Vec::new();
    for function in functions {
        // Check if this function is a method (has self parameter matching our target type)
        if let Some(method) = convert_function_to_method(db, function, target_type)? {
            methods.push(method);
        }
    }

    if methods.is_empty() {
        tracing::debug!(
            "Found {} functions but no methods for type {}",
            functions_count,
            target_type.display_name()
        );
        Ok(None)
    } else {
        tracing::debug!(
            "Found {} methods for type {}",
            methods.len(),
            target_type.display_name()
        );
        Ok(Some(methods))
    }
}

/// Convert a FunctionInfo to a DiscoveredMethod if it's a method for the target type
fn convert_function_to_method(
    db: &dyn rudy_dwarf::DwarfDb,
    function: rudy_dwarf::parser::functions::FunctionInfo,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
) -> Result<Option<DiscoveredMethod>> {
    // Check if first parameter matches our target type
    let Some(first_param) = function.parameters.first() else {
        return Ok(None); // No parameters, not a method
    };

    // Resolve the parameter type and check if it matches our target
    let param_type = rudy_dwarf::types::resolve_type_offset(db, first_param.type_die)?;
    if !param_type.matching_type(target_type) {
        return Ok(None); // First parameter doesn't match target type
    }

    // Determine the self type based on the parameter type
    use rudy_dwarf::function::SelfType;
    let self_type = SelfType::from_param_type(param_type.layout.as_ref());

    // This is a method! Convert to DiscoveredMethod
    // Note: We don't have the symbol address, so we'll mark as not callable
    Ok(Some(DiscoveredMethod {
        name: function.name.clone(),
        full_name: format!("{}::(direct)", function.name),
        signature: format!("fn {}", function.name), // Simplified signature
        address: 0,                                 // No symbol address available
        self_type,
        callable: false, // Not callable without symbol address
    }))
}

/// Phase 2: Discover trait implementations by searching for {impl#N} blocks
///
/// Rust trait implementations are compiled into {impl#N} modules adjacent to the type definition.
fn discover_trait_impl_methods(
    _db: &dyn Db,
    _binary: Binary,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
) -> Result<Vec<DiscoveredMethod>> {
    // This is a more complex implementation that would traverse the DWARF tree
    // looking for impl blocks. For now, we'll return an empty vec and implement
    // this in a future phase.
    tracing::debug!(
        "Trait implementation discovery not yet implemented for {}",
        target_type.display_name()
    );
    Ok(vec![])
}
