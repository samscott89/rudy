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

                let self_type = sig.self_type(db);
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
    if let Some(direct_methods) = discover_direct_methods(db, binary, target_type)? {
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
    db: &dyn Db,
    binary: Binary,
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

    // Get the symbol index for address lookup
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);

    let functions_count = functions.len();
    let mut methods = Vec::new();
    for function in functions {
        // Check if this function is a method (has self parameter matching our target type)
        if let Some(method) = convert_function_to_method(db, function, target_type, symbol_index)? {
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
    db: &dyn Db,
    function: rudy_dwarf::parser::functions::FunctionInfo,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
    symbol_index: &rudy_dwarf::symbols::SymbolIndex,
) -> Result<Option<DiscoveredMethod>> {
    // Determine if this is a method (has self parameter) or associated function
    use rudy_dwarf::function::SelfType;
    let self_type = if let Some(first_param) = function.parameters.first() {
        if first_param.name.as_ref().is_some_and(|n| n != "self") {
            // this is not a method - first parameter is not named "self"
            None
        } else {
            // Check if first parameter matches our target type
            let param_type = rudy_dwarf::types::resolve_type_offset(db, first_param.type_die)?;
            let param_layout = param_type.layout.dereferenced();
            if param_layout.matching_type(target_type.layout.as_ref()) {
                // This is a method with self parameter
                Some(SelfType::from_param_type(param_type.layout.as_ref()))
            } else {
                tracing::debug!(
                    "First parameter type {} does not match target type {} for function {}",
                    param_layout.display_name(),
                    target_type.display_name(),
                    function.name
                );
                // First parameter doesn't match target type - this could still be an associated function
                // For now, we'll include it as an associated function (no self)
                None
            }
        }
    } else {
        // No parameters - this is an associated function
        None
    };

    // Try to resolve the function address using linkage name
    let (address, callable, full_name) = if let Some(linkage_name) = &function.linkage_name {
        // Convert linkage name to RawSymbol and demangle it to get SymbolName
        let raw_symbol = rudy_dwarf::symbols::RawSymbol::new(linkage_name.as_bytes().to_vec());

        match raw_symbol.demangle() {
            Ok(symbol_name) => {
                if let Some(symbol) = symbol_index.get_function(&symbol_name) {
                    tracing::debug!(
                        "Found symbol address for method {}: {:#x}",
                        function.name,
                        symbol.address
                    );
                    (symbol.address, true, symbol.name.to_string())
                } else {
                    tracing::debug!(
                        "No symbol found for demangled linkage name: {} ({})",
                        symbol_name,
                        linkage_name
                    );
                    (0, false, format!("{}::(direct)", function.name))
                }
            }
            Err(e) => {
                tracing::debug!("Failed to demangle linkage name {}: {}", linkage_name, e);
                (0, false, format!("{}::(direct)", function.name))
            }
        }
    } else {
        tracing::debug!("No linkage name available for function: {}", function.name);
        (0, false, format!("{}::(direct)", function.name))
    };

    // Build a more complete signature
    let signature = build_function_signature(&function, self_type.as_ref());

    // This is a method! Convert to DiscoveredMethod
    Ok(Some(DiscoveredMethod {
        name: function.name.clone(),
        full_name,
        signature,
        address,
        self_type,
        callable,
    }))
}

/// Build a function signature string from FunctionInfo
fn build_function_signature(
    function: &rudy_dwarf::parser::functions::FunctionInfo,
    self_type: Option<&rudy_dwarf::function::SelfType>,
) -> String {
    let mut sig = String::new();
    sig.push_str("fn ");
    sig.push_str(&function.name);
    sig.push('(');

    let mut params = Vec::new();

    // Add self parameter if present
    if let Some(self_type) = self_type {
        params.push(match self_type {
            rudy_dwarf::function::SelfType::Owned => "self".to_string(),
            rudy_dwarf::function::SelfType::Borrowed => "&self".to_string(),
            rudy_dwarf::function::SelfType::BorrowedMut => "&mut self".to_string(),
        });
    }

    // Add other parameters (skip first if it's self)
    let skip_first = self_type.is_some();
    for (i, param) in function.parameters.iter().enumerate() {
        if skip_first && i == 0 {
            continue;
        }

        let param_str = if let Some(name) = &param.name {
            format!("{name}: _")
        } else {
            "_: _".to_string()
        };
        params.push(param_str);
    }

    sig.push_str(&params.join(", "));
    sig.push(')');

    // Add return type if present
    if function.return_type.is_some() {
        sig.push_str(" -> _");
    }

    sig
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
