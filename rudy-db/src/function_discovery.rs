use std::collections::BTreeMap;

use anyhow::Result;
use rudy_dwarf::{
    Binary, Die, SymbolName,
    function::{FunctionSignature, resolve_function_signature},
    gimli,
    parser::{
        Parser,
        children::for_each_child,
        combinators::all,
        functions::function_parser,
        primitives::{is_member_tag, resolve_type},
    },
    types::DieTypeDefinition,
};
use rudy_types::Layout;

use crate::{DiscoveredMethod, FunctionParameter, database::Db};

/// Result of analyzing a symbol for method discovery
#[derive(Debug, Clone)]
pub enum SymbolAnalysisResult {
    /// Successfully discovered a method
    DiscoveredFunction {
        signature: FunctionSignature,
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
                let params = &sig.params;
                let Some(first_param) = params.first() else {
                    // no parameters, skip
                    continue;
                };

                let self_type = sig.self_type;
                let first_type = first_param.ty.layout.dereferenced();

                if matches!(first_type, Layout::Alias { name } if name == "unknown") {
                    // Skip methods with an unknown type
                    continue;
                }

                methods_by_type
                    .entry(first_type.display_name())
                    .or_default()
                    .push(DiscoveredMethod {
                        name: sig.name.to_string(),
                        full_name: symbol.name.to_string(),
                        signature: sig.print_sig(),
                        address: symbol.address,
                        self_type,
                        // defaults to callable
                        callable: true,
                        is_synthetic: false,
                        return_type: sig.return_type,
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

    // Phase 3: Add synthetic methods based on the type layout
    let synthetic_methods =
        crate::synthetic_methods::get_synthetic_methods(target_type.layout.as_ref());
    for synthetic in synthetic_methods {
        methods.push(DiscoveredMethod {
            name: synthetic.name.to_string(),
            full_name: format!("{}::{}", target_type.display_name(), synthetic.name),
            signature: synthetic.signature.to_string(),
            address: 0, // Synthetic methods don't have addresses
            self_type: Some(rudy_dwarf::function::SelfType::Borrowed), // Most synthetic methods take &self
            callable: false, // Can't call synthetic methods directly
            is_synthetic: true,
            return_type: None, // Synthetic methods don't have DWARF type definitions
        });
    }

    tracing::debug!(
        "Final method count for type {} (including synthetic): {}",
        target_type.display_name(),
        methods.len()
    );
    Ok(methods)
}

/// Find all DIEs that represent the same type as target_type across all compilation units
fn find_matching_type_dies(
    db: &dyn Db,
    binary: Binary,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
) -> Result<Vec<rudy_dwarf::Die>> {
    let mut matching_dies = vec![];

    // Get the type's namespace path for searching
    // find the module path

    let Some(module_path) = rudy_dwarf::modules::get_containing_module(db, target_type.location)
    else {
        tracing::warn!(
            "No module index found for type {}, returning only original DIE",
            target_type.display_name()
        );
        matching_dies.push(target_type.location);
        return Ok(matching_dies);
    };

    /// Parser to find all children of a type DIE that match the target type
    fn is_matching_type(target_type: &DieTypeDefinition) -> impl Parser<Vec<Die>> {
        for_each_child(
            all((
                is_member_tag(gimli::DW_TAG_structure_type),
                resolve_type().map_res(|t| {
                    if t.layout.matching_type(target_type.layout.as_ref()) {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "Type {} does not match target type {}",
                            t.layout.display_name(),
                            target_type.display_name()
                        ))
                    }
                }),
            ))
            .map(|(die, _)| die),
        )
    }

    // find all DIEs across all debug files that match the namespace
    let debug_files = crate::index::debug_index(db, binary).debug_files(db);
    for debug_file in debug_files.values() {
        let Some(module) =
            rudy_dwarf::modules::module_index(db, *debug_file).find_by_path(db, &module_path)
        else {
            tracing::trace!(
                "No module found for path {} in debug file {}",
                module_path.join("::"),
                debug_file.name(db)
            );
            continue;
        };

        tracing::trace!(
            "{} modules found with a matching path",
            module.entries.len(),
        );

        for module_die in &module.entries {
            tracing::trace!(
                "In module: {} at {}",
                module_die.name(db).unwrap(),
                module_die.location(db)
            );
            // need to find all children that are matching structs
            matching_dies.extend(is_matching_type(target_type).parse(db, *module_die)?)
        }
    }

    Ok(matching_dies)
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

    // Find all DIEs that represent the same type (might be spread across compilation units)
    let matching_type_dies = find_matching_type_dies(db, binary, target_type)?;

    if matching_type_dies.is_empty() {
        tracing::trace!(
            "No matching type DIEs found for type {}",
            target_type.display_name()
        );
        return Ok(None);
    }

    tracing::trace!(
        "Found {} type DIEs for type {}",
        matching_type_dies.len(),
        target_type.display_name()
    );

    // Get the symbol index for address lookup
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);

    let mut all_methods = Vec::new();

    // Process each matching type DIE
    for type_die in matching_type_dies {
        // Use the function parser to find all functions nested under this type DIE
        let functions = child_functions_parser().parse(db, type_die)?;

        tracing::trace!(
            "Type DIE at {}: found {} functions",
            type_die.location(db),
            functions.len()
        );

        for function in functions {
            tracing::trace!(
                "  - Function {} in DIE at {}",
                function.name,
                type_die.location(db)
            );
            // Check if this function is a method (has self parameter matching our target type)
            if let Some(method) =
                convert_function_to_method(db, function, target_type, symbol_index)?
            {
                all_methods.push(method);
            }
        }
    }

    if all_methods.is_empty() {
        tracing::debug!("No methods found for type {}", target_type.display_name());
        Ok(None)
    } else {
        tracing::debug!(
            "Found {} total methods for type {}",
            all_methods.len(),
            target_type.display_name()
        );
        Ok(Some(all_methods))
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
    let signature = build_function_signature(db, &function, self_type.as_ref());

    // Get the return type definition if available
    let return_type_def = if let Some(return_die) = function.return_type {
        rudy_dwarf::types::resolve_type_offset(db, return_die).ok()
    } else {
        None
    };

    // This is a method! Convert to DiscoveredMethod
    Ok(Some(DiscoveredMethod {
        name: function.name.clone(),
        full_name,
        signature,
        address,
        self_type,
        callable,
        is_synthetic: false,
        return_type: return_type_def,
    }))
}

/// Build a function signature string from FunctionInfo
fn build_function_signature(
    db: &dyn Db,
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

        // Try to resolve the parameter type
        let type_str = match rudy_dwarf::types::resolve_type_offset(db, param.type_die) {
            Ok(resolved_type) => resolved_type.display_name(),
            Err(_) => "?".to_string(),
        };

        let param_str = if let Some(name) = &param.name {
            format!("{name}: {type_str}")
        } else {
            format!("_: {type_str}")
        };
        params.push(param_str);
    }

    sig.push_str(&params.join(", "));
    sig.push(')');

    // Add return type if present
    if let Some(return_die) = function.return_type {
        match rudy_dwarf::types::resolve_type_offset(db, return_die) {
            Ok(resolved_type) => {
                let return_type = resolved_type.display_name();
                if return_type != "()" {
                    // Don't show unit type
                    sig.push_str(" -> ");
                    sig.push_str(&return_type);
                }
            }
            Err(_) => {
                sig.push_str(" -> ?");
            }
        }
    }

    sig
}

/// Phase 2: Discover trait implementations by searching for {impl#N} blocks
///
/// Rust trait implementations are compiled into {impl#N} modules adjacent to the type definition.
fn discover_trait_impl_methods(
    db: &dyn Db,
    binary: Binary,
    target_type: &rudy_dwarf::types::DieTypeDefinition,
) -> Result<Vec<DiscoveredMethod>> {
    use rudy_dwarf::parser::Parser;
    use rudy_dwarf::parser::functions::{child_functions_parser, impl_namespaces_in_module_parser};

    let type_die = target_type.location;

    // Find all {impl#N} namespaces in the same compilation unit
    let impl_namespaces = impl_namespaces_in_module_parser().parse(db, type_die)?;

    if impl_namespaces.is_empty() {
        tracing::debug!(
            "No impl namespaces found for type {}",
            target_type.display_name()
        );
        return Ok(vec![]);
    }

    // Get the symbol index for address lookup
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);

    let mut trait_methods = Vec::new();

    for impl_namespace in impl_namespaces {
        tracing::debug!("Searching impl namespace for trait methods");

        // Find all functions within this impl namespace
        let functions = child_functions_parser().parse(db, impl_namespace)?;

        // Check if ANY function in this impl block is for our target type
        // If none are, skip this entire impl block
        let mut impl_methods = Vec::new();
        for function in functions {
            // Check if this function is a method for our target type
            if let Some(method) =
                convert_function_to_method(db, function, target_type, symbol_index)?
            {
                // For trait methods, only include methods that have self parameters
                // Skip associated functions (no self) as they don't make sense for method discovery on pointers
                if method.self_type.is_some() {
                    impl_methods.push(method);
                } else {
                    tracing::debug!(
                        "Skipping trait associated function {} (no self parameter)",
                        method.name
                    );
                }
            }
        }

        // Only add methods from impl blocks that actually contain methods for our target type
        if !impl_methods.is_empty() {
            tracing::debug!(
                "Found {} methods for target type in impl block",
                impl_methods.len()
            );

            for method in impl_methods {
                // Enhance the method with trait context
                let enhanced_method = enhance_method_with_trait_info(method, &impl_namespace, db)?;
                trait_methods.push(enhanced_method);
            }
        } else {
            tracing::debug!("Impl block contains no methods for target type, skipping");
        }
    }

    tracing::debug!(
        "Found {} trait methods for type {}",
        trait_methods.len(),
        target_type.display_name()
    );

    Ok(trait_methods)
}

/// Enhance a discovered method with trait information from symbol demangling
fn enhance_method_with_trait_info(
    mut method: DiscoveredMethod,
    _impl_namespace: &rudy_dwarf::Die,
    _db: &dyn rudy_dwarf::DwarfDb,
) -> Result<DiscoveredMethod> {
    // Try to extract trait information from the demangled symbol name
    if let Ok(demangled) = extract_trait_from_symbol(&method.full_name) {
        // Update the full name to include trait context
        method.full_name = format!("{} (from {})", method.full_name, demangled);
    } else {
        // Fallback: just indicate it's from a trait impl
        method.full_name = format!("{} (from trait impl)", method.full_name);
    }

    Ok(method)
}

/// Extract trait name from demangled symbol name
/// Example: "Session as Describable" from symbol like "_ZN75_$LT$method_discovery..Session$u20$as$u20$method_discovery..Describable$GT$..."
fn extract_trait_from_symbol(symbol_name: &str) -> Result<String> {
    // This is a simplified trait extraction
    // In a full implementation, we'd use a proper Rust symbol demangling library
    // For now, we'll look for patterns in the symbol name

    if symbol_name.contains(" as ") {
        // Already demangled, extract the trait part
        if let Some(as_pos) = symbol_name.find(" as ") {
            let after_as = &symbol_name[as_pos + 4..];
            if let Some(end_pos) = after_as.find("::") {
                return Ok(after_as[..end_pos].to_string());
            } else {
                return Ok(after_as.to_string());
            }
        }
    }

    // If no trait information can be extracted, return an error
    Err(anyhow::anyhow!("No trait information found in symbol name"))
}

/// Discover all functions in the binary that match a given pattern
///
/// This function iterates through all function symbols in the binary and matches
/// them against the provided pattern. It supports:
/// - Exact matches (e.g., "main")
/// - Fuzzy matches (e.g., "calc" matching "calculate_sum")
/// - Fully qualified names (e.g., "test_mod1::my_fn")
///
/// Returns a vector of discovered functions sorted by match quality.
pub fn discover_functions(
    db: &dyn Db,
    binary: Binary,
    pattern: &SymbolName,
) -> Result<Vec<crate::DiscoveredFunction>> {
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);

    let mut discovered_functions = Vec::new();

    // Iterate through all function symbols
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        // Check if this function matches the pattern
        if function_matches_pattern(&symbol.name, pattern) {
            // Try to get debug info for this function
            if let Some((debug_file, fie)) = index.get_function(db, &symbol.name) {
                match create_discovered_function(db, symbol, fie, debug_file) {
                    Ok(discovered_func) => discovered_functions.push(discovered_func),
                    Err(e) => {
                        tracing::debug!(
                            "Failed to create discovered function for {}: {}",
                            symbol.name,
                            e
                        );
                        // Still add it as a basic function without detailed info
                        discovered_functions.push(crate::DiscoveredFunction {
                            name: symbol.name.lookup_name.clone(),
                            full_name: symbol.name.to_string(),
                            signature: format!("fn {}(?)", symbol.name),
                            address: symbol.address,
                            callable: false,
                            module_path: symbol.name.module_path.clone(),
                            return_type: None,
                            parameters: vec![],
                        });
                    }
                }
            } else {
                // Function symbol exists but no debug info
                discovered_functions.push(crate::DiscoveredFunction {
                    name: symbol.name.lookup_name.clone(),
                    full_name: symbol.name.to_string(),
                    signature: format!("fn {}(?)", symbol.name),
                    address: symbol.address,
                    callable: false,
                    module_path: symbol.name.module_path.clone(),
                    return_type: None,
                    parameters: vec![],
                });
            }
        }
    }

    Ok(discovered_functions)
}

fn function_matches_pattern(symbol_name: &SymbolName, pattern: &SymbolName) -> bool {
    // Check for exact match
    if symbol_name == pattern {
        return true;
    }

    // Check for suffix match
    if symbol_name.lookup_name == pattern.lookup_name
        && symbol_name.module_path.ends_with(&pattern.module_path)
    {
        return true;
    }

    false
}

/// Discover all functions in the binary
///
/// Returns a map of function name to discovered function information.
pub fn discover_all_functions(
    db: &dyn Db,
    binary: Binary,
) -> Result<BTreeMap<String, crate::DiscoveredFunction>> {
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);

    let mut discovered_functions = BTreeMap::new();

    // Iterate through all function symbols
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        let symbol_name = symbol.name.to_string();

        // Try to get debug info for this function
        let discovered_func = if let Some((debug_file, fie)) = index.get_function(db, &symbol.name)
        {
            match create_discovered_function(db, symbol, fie, debug_file) {
                Ok(func) => func,
                Err(e) => {
                    tracing::debug!(
                        "Failed to create discovered function for {}: {}",
                        symbol_name,
                        e
                    );
                    // Still add it as a basic function without detailed info
                    crate::DiscoveredFunction {
                        name: symbol.name.lookup_name.clone(),
                        full_name: symbol_name.clone(),
                        signature: format!("fn {}(?)", symbol.name),

                        address: symbol.address,
                        callable: false,
                        module_path: symbol.name.module_path.clone(),
                        return_type: None,
                        parameters: vec![],
                    }
                }
            }
        } else {
            // Function symbol exists but no debug info
            crate::DiscoveredFunction {
                name: symbol.name.lookup_name.clone(),
                full_name: symbol_name.clone(),
                signature: format!("fn {}(?)", symbol.name),
                address: symbol.address,
                callable: false,
                module_path: symbol.name.module_path.clone(),
                return_type: None,
                parameters: vec![],
            }
        };

        discovered_functions.insert(symbol_name, discovered_func);
    }

    Ok(discovered_functions)
}

/// Create a discovered function from symbol and debug info
fn create_discovered_function<'db>(
    db: &'db dyn Db,
    symbol: &rudy_dwarf::symbols::Symbol,
    fie: rudy_dwarf::function::FunctionIndexEntry<'db>,
    _debug_file: rudy_dwarf::DebugFile,
) -> Result<crate::DiscoveredFunction> {
    let symbol_name = symbol.name.to_string();

    // Get function data from the index entry
    let fie_data = fie.data(db);

    let f = function_parser().parse(db, fie_data.declaration_die)?;

    let params: Vec<_> = f
        .parameters
        .into_iter()
        .map(|p| {
            Ok::<_, anyhow::Error>(FunctionParameter {
                name: p.name.clone(),
                type_def: rudy_dwarf::types::resolve_type_offset(db, p.type_die)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let return_type = if let Some(return_die) = f.return_type {
        Some(rudy_dwarf::types::resolve_type_offset(db, return_die)?)
    } else {
        None
    };

    // Build signature
    let signature =
        build_function_signature_for_discovered(&symbol.name, &params, return_type.as_ref());

    Ok(crate::DiscoveredFunction {
        name: symbol.name.lookup_name.clone(),
        full_name: symbol_name.clone(),
        signature,
        address: symbol.address,
        callable: true,
        module_path: symbol.name.module_path.clone(),
        return_type,
        parameters: params,
    })
}

/// Build a function signature string from parameters and return type
fn build_function_signature_for_discovered(
    symbol: &SymbolName,
    parameters: &[crate::FunctionParameter],
    return_type: Option<&DieTypeDefinition>,
) -> String {
    let mut sig = String::new();
    sig.push_str("fn ");
    sig.push_str(&symbol.lookup_name);
    sig.push('(');

    let param_strings: Vec<String> = parameters
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let type_str = param.type_def.display_name();
            if let Some(name) = &param.name {
                format!("{name}: {type_str}")
            } else {
                format!("_{i}: {type_str}")
            }
        })
        .collect();

    sig.push_str(&param_strings.join(", "));
    sig.push(')');

    // Add return type if present and not unit type
    if let Some(ret_type) = return_type {
        let return_type_str = ret_type.display_name();
        if return_type_str != "()" {
            sig.push_str(" -> ");
            sig.push_str(&return_type_str);
        }
    }

    sig
}
