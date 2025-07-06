//! Function parsing combinators for DWARF debug information

use super::children::for_each_child;
use super::primitives::entry_type;
use super::{from_fn, Parser};
use crate::parser::combinators::all;
use crate::parser::primitives::{is_member_tag, name};
use crate::{Die, DwarfDb};

/// Information about a function discovered in DWARF
#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub name: String,
    pub linkage_name: Option<String>,
    pub die: Die,
    pub parameters: Vec<ParameterInfo>,
    pub return_type: Option<Die>,
}

/// Information about a function parameter
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: Option<String>,
    pub type_die: Die,
}

/// Parser that attempts to parse a DIE as a function
pub fn function_parser() -> impl Parser<Option<FunctionInfo>> {
    from_fn(
        |db: &dyn DwarfDb, entry: Die| -> anyhow::Result<Option<FunctionInfo>> {
            // Check if this is a function DIE
            if entry.tag(db) != crate::gimli::DW_TAG_subprogram {
                return Ok(None);
            }

            // Parse function components using parser combinators
            let name = entry.name(db).unwrap_or_else(|_| "<anonymous>".to_string());

            // Extract linkage name for symbol lookup
            let linkage_name = entry.string_attr(db, crate::gimli::DW_AT_linkage_name).ok();

            // Parse return type (optional)
            let return_type = entry_type().parse(db, entry).ok();

            // Parse parameters from children
            let parameters = parameter_list_parser().parse(db, entry)?;

            Ok(Some(FunctionInfo {
                name,
                linkage_name,
                die: entry,
                parameters,
                return_type,
            }))
        },
    )
}

/// Parser for function parameters
fn parameter_parser() -> impl Parser<Option<ParameterInfo>> {
    from_fn(
        |db: &dyn DwarfDb, entry: Die| -> anyhow::Result<Option<ParameterInfo>> {
            // Only process formal parameter DIEs
            if entry.tag(db) != crate::gimli::DW_TAG_formal_parameter {
                return Ok(None);
            }

            let name = entry.name(db).ok();
            let type_die = entry_type().parse(db, entry)?;

            Ok(Some(ParameterInfo { name, type_die }))
        },
    )
}

/// Parser that extracts all parameters from children
fn parameter_list_parser() -> impl Parser<Vec<ParameterInfo>> {
    for_each_child(parameter_parser()).map(|results| results.into_iter().flatten().collect())
}

/// Parser that finds all functions among the children of a DIE
pub fn child_functions_parser() -> impl Parser<Vec<FunctionInfo>> {
    for_each_child(function_parser()).map(|results| results.into_iter().flatten().collect())
}

/// Parser that finds impl namespace DIEs containing trait implementations
/// by looking for {impl#N} patterns that are siblings to the target type
pub fn impl_namespaces_in_module_parser() -> impl Parser<Vec<Die>> {
    from_fn(
        |db: &dyn DwarfDb, type_die: Die| -> anyhow::Result<Vec<Die>> {
            let mut impl_namespaces = Vec::new();

            // Find {impl#N} blocks that are siblings to our target type
            // This is more targeted than searching the entire compilation unit
            find_sibling_impl_namespaces(db, type_die, &mut impl_namespaces)?;

            tracing::debug!(
                "Found {} impl namespaces as siblings to target type",
                impl_namespaces.len()
            );
            Ok(impl_namespaces)
        },
    )
}

/// Helper function to find {impl#N} namespaces that are in the same module as the target type
/// Uses module indexing to find DIEs in the same namespace scope
fn find_sibling_impl_namespaces(
    db: &dyn DwarfDb,
    type_die: Die,
    impl_namespaces: &mut Vec<Die>,
) -> anyhow::Result<()> {
    // Get the module index for this debug file
    let module_index = crate::modules::module_index(db, type_die.file);

    // Find the module path for our target type using its DIE offset
    let target_offset = type_die.offset();
    let Some(module_range) = module_index.find_by_offset(db, target_offset) else {
        tracing::debug!("No module range found for offset {target_offset:#x}");
        return Ok(());
    };

    tracing::debug!(
        "Target type is in module path: {:?} and DIE: {}",
        module_range.module_path,
        module_range.die.print(db)
    );

    // find all impl namespaces in the same module
    let impls = for_each_child(
        all((
            is_member_tag(gimli::DW_TAG_namespace),
            name()
                .map_with_entry(|_, entry, n| {
                    n.and_then(|n| {
                        if n.starts_with("{impl#") && n.ends_with('}') {
                            tracing::debug!("Found impl namespace: {n}");
                            Some(entry)
                        } else {
                            None
                        }
                    })
                })
                .map_res(|entry| {
                    entry.ok_or_else(|| anyhow::anyhow!("Expected impl namespace DIE, found None"))
                }),
        ))
        .map(|(die, _)| die),
    )
    .parse(db, module_range.die)?;

    impl_namespaces.extend(impls);

    Ok(())
}
