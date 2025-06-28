use anyhow::{Context, Result};
use rudy_types::{PrimitiveLayout, ReferenceLayout, TypeLayout, UnitLayout};
use std::{collections::BTreeMap, fmt, sync::Arc};

use crate::{
    ResolvedLocation,
    database::{Db, Diagnostic, handle_diagnostics},
    dwarf::{self, Die, resolve_function_variables},
    file::Binary,
    index,
    outputs::ResolvedFunction,
    query::{lookup_address, lookup_position},
    types::{Address, Position},
};

/// Result of analyzing a symbol for method discovery
#[derive(Debug, Clone, serde::Serialize)]
pub enum SymbolAnalysisResult {
    /// Successfully discovered a method
    DiscoveredMethod {
        self_type: String,
        method: DiscoveredFunction,
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
}

/// Information about a method discovered from debug information
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredFunction {
    /// Short method name (e.g., "len")
    pub name: String,
    /// Full qualified function name (e.g., "std::vec::Vec<T>::len")
    pub full_name: String,
    /// Method signature
    pub signature: String,
    /// Address in binary where method is located
    pub address: u64,
    /// Type of self parameter
    pub self_type: Option<SelfType>,
    /// Number of parameters including self
    pub parameter_count: usize,
    /// Whether this method can be called (has symbol)
    pub callable: bool,
}

/// Type of self parameter in a method
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub enum SelfType {
    /// `self` - takes ownership
    Owned,
    /// `&self` - immutable reference
    Borrowed,
    /// `&mut self` - mutable reference
    BorrowedMut,
}

impl SelfType {
    pub fn from_param_type(param_type: &TypeLayout) -> Self {
        match param_type {
            TypeLayout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: true,
                ..
            })) => Self::BorrowedMut,
            TypeLayout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: false,
                ..
            })) => Self::Borrowed,
            _ => Self::Owned,
        }
    }
}

/// Discover actual methods for a type from DWARF debug information
///
/// This searches through all functions in the debug info to find methods
/// that operate on the given type (functions with &self, &mut self, or self parameters).
///
/// # Arguments
///
/// * `target_type` - The type to find methods for
///
/// # Returns
///
/// A list of discovered methods with their signatures
pub fn discover_methods_for_type<'db>(
    db: &'db dyn Db,
    binary: Binary,
    target_type: &TypeLayout,
) -> Result<Vec<DiscoveredFunction>> {
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);
    let mut discovered_methods = Vec::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        // Check if this function exists in DWARF debug info
        if let Some((debug_file, function_entry)) = index.get_function(db, &symbol.name) {
            let return_type = crate::dwarf::resolve_entry_type(db, function_entry.declaration_die)
                .unwrap_or(TypeLayout::Primitive(PrimitiveLayout::Unit(UnitLayout)));

            // Get the function's DIE (prefer specification over declaration for variable resolution)
            let function_die = function_entry
                .specification_die
                .unwrap_or(function_entry.declaration_die);

            // Resolve function variables to get parameters

            let variables = crate::dwarf::resolve_function_variables(db, function_die)?;
            // we'll check error cases and log them here
            let diagnostics: Vec<&Diagnostic> =
                crate::dwarf::resolve_function_variables::accumulated(db, function_die);
            handle_diagnostics(&diagnostics)?;
            let parameters = variables.params(db);

            // Check if this is a method for our target type
            if let Some(method) = analyze_function_as_method(
                db,
                target_type,
                &symbol.name.to_string(),
                parameters,
                &return_type,
                symbol,
                debug_file,
            )
            .unwrap_or_else(|e| {
                tracing::error!("Failed to analyze function '{}': {e}", symbol.name);
                None
            }) {
                discovered_methods.push(method);
            }
        }
    }

    // Add synthetic methods that can be evaluated without execution
    let synthetic_methods = crate::synthetic_methods::get_synthetic_methods(target_type);
    for synthetic in synthetic_methods {
        discovered_methods.push(DiscoveredFunction {
            name: synthetic.name.to_string(),
            full_name: format!("{}::{}", target_type.display_name(), synthetic.name),
            signature: synthetic.signature.to_string(),
            address: 0,                          // Synthetic methods don't have addresses
            self_type: Some(SelfType::Borrowed), // Most synthetic methods take &self
            parameter_count: if synthetic.takes_args { 1 } else { 0 }, // Simplified for now
            callable: false,                     // Synthetic methods aren't callable via execution
        });
    }

    Ok(discovered_methods)
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
    let symbol_index = index.symbol_index(db);
    let mut symbol_results = BTreeMap::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        let symbol_name = symbol.name.to_string();

        let Some((debug_file, function_entry)) = index.get_function(db, &symbol.name) else {
            symbol_results.insert(symbol_name, SymbolAnalysisResult::NoDebugInfo);
            continue;
        };

        // return type lives in the declaration DIE
        let return_type =
            match crate::dwarf::resolve_entry_type_optional(db, function_entry.declaration_die) {
                Ok(Some(ty)) => ty,
                Ok(None) => TypeLayout::Primitive(PrimitiveLayout::Unit(UnitLayout)),
                Err(e) => {
                    symbol_results.insert(
                        symbol_name.clone(),
                        SymbolAnalysisResult::TypeResolutionError {
                            error: format!("Failed to resolve return type: {e}"),
                        },
                    );
                    continue;
                }
            };

        // variable types will likely be in the specification DIE, but if not, use declaration DIE
        // to resolve function variables
        let function_die = function_entry
            .specification_die
            .unwrap_or(function_entry.declaration_die);

        // Try to resolve function variables
        let result = match crate::dwarf::resolve_function_variables(db, function_die) {
            Ok(variables) => {
                let parameters = variables.params(db);

                // Try to analyze as method
                match analyze_function_for_any_method(
                    db,
                    &symbol_name,
                    parameters,
                    &return_type,
                    symbol,
                    debug_file,
                ) {
                    Ok(Some((self_type, method))) => SymbolAnalysisResult::DiscoveredMethod {
                        self_type: self_type.dereferenced().display_name(),
                        method,
                    },
                    Ok(None) => SymbolAnalysisResult::NotAMethod {
                        reason: "No self parameter or type mismatch".to_string(),
                    },
                    Err(e) => SymbolAnalysisResult::AnalysisError {
                        error: format!("Method analysis failed: {e}"),
                    },
                }
            }
            Err(e) => SymbolAnalysisResult::VariableResolutionError {
                error: format!("Failed to resolve function variables: {e}"),
            },
        };

        symbol_results.insert(symbol_name, result);
    }

    Ok(symbol_results)
}

pub fn discover_all_methods<'db>(
    db: &'db dyn Db,
    binary: Binary,
) -> Result<BTreeMap<String, Vec<DiscoveredFunction>>> {
    let index = crate::index::debug_index(db, binary);
    let symbol_index = index.symbol_index(db);
    let mut methods_by_type = BTreeMap::new();

    // Iterate through all functions that have symbols in the binary
    for symbol in symbol_index.functions.values().flat_map(|map| map.values()) {
        // Check if this function exists in DWARF debug info
        if let Some((debug_file, function_entry)) = index.get_function(db, &symbol.name) {
            let return_type = crate::dwarf::resolve_entry_type(db, function_entry.declaration_die)
                .unwrap_or(TypeLayout::Primitive(PrimitiveLayout::Unit(UnitLayout)));

            // Get the function's DIE (prefer specification over declaration for parameter resolution)
            let function_die = function_entry
                .specification_die
                .unwrap_or(function_entry.declaration_die);

            // Resolve function variables to get parameters
            if let Ok(variables) = crate::dwarf::resolve_function_variables(db, function_die) {
                let parameters = variables.params(db);

                // Check if this is a method (has self parameter)
                if let Some((self_type, method)) = analyze_function_for_any_method(
                    db,
                    &symbol.name.to_string(),
                    parameters,
                    &return_type,
                    symbol,
                    debug_file,
                )
                .unwrap_or_else(|e| {
                    tracing::error!("Failed to analyze function '{}': {e}", symbol.name);
                    None
                }) {
                    let type_name = self_type.dereferenced().display_name();
                    methods_by_type
                        .entry(type_name)
                        .or_insert_with(Vec::new)
                        .push(method);
                }
            }
        }
    }

    Ok(methods_by_type)
}

/// Analyze a function to see if it's a method for the target type
fn analyze_function_as_method<'db>(
    db: &'db dyn Db,
    target_type: &TypeLayout,
    function_name: &str,
    parameters: Vec<crate::dwarf::Variable>,
    return_type: &TypeLayout,
    symbol: &crate::index::symbols::Symbol,
    debug_file: crate::file::DebugFile,
) -> Result<Option<DiscoveredFunction>> {
    // Must have at least one parameter to be a method
    if parameters.is_empty() {
        return Ok(None);
    }

    // Get the first parameter (potential self parameter)
    let first_param = &parameters[0];
    let first_param_name = first_param.name(db);

    // Check if first parameter is self-like
    if !matches!(first_param_name.as_str(), "self" | "&self" | "&mut self") {
        return Ok(None);
    }

    // Resolve the first parameter's type
    let param_type = first_param.ty(db);
    let resolved_param_type = crate::dwarf::fully_resolve_type(db, debug_file, param_type)?;

    // Check if the parameter type matches our target type
    if !target_type.matching_type(resolved_param_type.dereferenced()) {
        return Ok(None);
    }

    // Extract method name from full function name
    let method_name = extract_method_name(function_name);

    // Build method signature
    let signature = build_method_signature(db, function_name, &parameters, return_type)?;

    Ok(Some(DiscoveredFunction {
        name: method_name,
        full_name: function_name.to_string(),
        signature,
        address: symbol.address,
        self_type: Some(SelfType::from_param_type(&resolved_param_type)),
        parameter_count: parameters.len(),
        callable: true, // Has symbol, so it's callable
    }))
}

/// Analyze a function to see if it's a method for any type
fn analyze_function_for_any_method<'db>(
    db: &'db dyn Db,
    function_name: &str,
    parameters: Vec<crate::dwarf::Variable>,
    return_type: &TypeLayout,
    symbol: &crate::index::symbols::Symbol,
    debug_file: crate::file::DebugFile,
) -> Result<Option<(TypeLayout, DiscoveredFunction)>> {
    // Must have at least one parameter to be a method
    if parameters.is_empty() {
        return Ok(None);
    }

    // Get the first parameter (potential self parameter)
    let first_param = &parameters[0];
    let first_param_name = first_param.name(db);

    // Check if first parameter is self-like
    if !matches!(first_param_name.as_str(), "self" | "&self" | "&mut self") {
        return Ok(None);
    }

    // Resolve the first parameter's type
    let param_type = first_param.ty(db);
    let resolved_param_type = crate::dwarf::fully_resolve_type(db, debug_file, param_type)?;

    // Extract method name from full function name
    let method_name = extract_method_name(function_name);

    // Build method signature
    let signature = build_method_signature(db, function_name, &parameters, return_type)?;

    let method = DiscoveredFunction {
        name: method_name,
        full_name: function_name.to_string(),
        signature,
        address: symbol.address,
        self_type: Some(SelfType::from_param_type(&resolved_param_type)),
        parameter_count: parameters.len(),
        callable: true, // Has symbol, so it's callable
    };

    Ok(Some((resolved_param_type, method)))
}

/// Extract the method name from a full function name
/// e.g., "std::vec::Vec<T>::len" -> "len"
fn extract_method_name(full_name: &str) -> String {
    full_name
        .split("::")
        .last()
        .unwrap_or(full_name)
        .to_string()
}

/// Build a method signature from function name and parameters
fn build_method_signature<'db>(
    db: &'db dyn Db,
    _function_name: &str,
    parameters: &[crate::dwarf::Variable],
    return_type: &TypeLayout,
) -> Result<String> {
    let mut signature = String::from("fn(");

    for (i, param) in parameters.iter().enumerate() {
        if i > 0 {
            signature.push_str(", ");
        }

        let param_name = param.name(db);
        let param_type = param.ty(db);

        signature.push_str(&format!("{}: {}", param_name, param_type.display_name()));
    }

    signature.push(')');

    if !matches!(return_type, TypeLayout::Primitive(PrimitiveLayout::Unit(_))) {
        signature.push_str(" -> ");
        signature.push_str(&return_type.display_name());
    }

    Ok(signature)
}
