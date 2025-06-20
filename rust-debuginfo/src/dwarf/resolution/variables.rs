//! Variable resolution from DWARF debugging information

use crate::database::Db;
use crate::dwarf::die::declaration_file;
use crate::dwarf::{Die, resolution::types::resolve_type_offset};
use crate::file::SourceFile;
use crate::typedef::TypeDef;

type Result<T> = std::result::Result<T, super::Error>;

/// Tracked variable information
#[salsa::tracked]
pub struct Variable<'db> {
    #[returns(ref)]
    pub name: String,
    #[returns(ref)]
    pub ty: TypeDef,
    pub file: SourceFile<'db>,
    pub line: u64,
    pub origin: Die<'db>,
}

/// Resolved variables for a function
#[salsa::tracked]
pub struct ResolvedVariables<'db> {
    pub params: Vec<Variable<'db>>,
    pub locals: Vec<Variable<'db>>,
}

/// Resolve all variables in a function
///
/// TODO(Sam): I think I should move globals outside of this
/// since those generally happen at the top level and don't
/// necessarily appear in this function's DIE.
#[salsa::tracked]
pub fn resolve_function_variables<'db>(
    db: &'db dyn Db,
    die: Die<'db>,
) -> Result<ResolvedVariables<'db>> {
    tracing::debug!(
        "{}",
        die.format_with_location(db, "resolving function variables")
    );

    let mut params = vec![];
    let mut locals = vec![];

    // recurse function to find params + locals
    for child in die.children(db) {
        match child.tag(db) {
            gimli::DW_TAG_formal_parameter => {
                tracing::debug!("parameter: {}", child.print(db));
                let param = resolve_function_parameter_entry(db, child)?;
                params.push(param);
            }
            gimli::DW_TAG_variable => {
                tracing::debug!("variable: {}", child.print(db));

                for grandchild in child.children(db) {
                    tracing::debug!("variable child: {}", grandchild.print(db));
                }
            }
            gimli::DW_TAG_lexical_block => {
                tracing::debug!("block: {}", child.print(db));
                for grandchild in child.children(db) {
                    if grandchild.tag(db) == gimli::DW_TAG_variable {
                        tracing::debug!("variable child: {}", grandchild.print(db));
                        let var = resolve_variable_entry(db, grandchild)?;
                        locals.push(var);
                    } else {
                        tracing::debug!("other block child: {}", grandchild.print(db));
                    }
                }
            }
            t => {
                // not a variable -- skip it
                tracing::debug!("skipping non-variable entry: {t}: {}", child.print(db));
            }
        }
    }
    Ok(ResolvedVariables::new(db, params, locals))
}

fn resolve_function_parameter_entry<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
) -> Result<Variable<'db>> {
    let Some(name) = entry.name(db) else {
        return Err(entry
            .format_with_location(db, "Failed to get parameter name")
            .into());
    };

    let Some(type_entry) = entry.get_unit_ref(db, gimli::DW_AT_type)? else {
        return Err(entry
            .format_with_location(db, "Failed to get type for parameter")
            .into());
    };

    let ty = resolve_type_offset(db, type_entry)?;

    let Some(file) = declaration_file(db, entry) else {
        return Err(entry
            .format_with_location(db, "Failed to get declaration file for parameter")
            .into());
    };
    let Some(line) = entry
        .get_attr(db, gimli::DW_AT_decl_line)
        .and_then(|v| v.udata_value())
    else {
        return Err(entry
            .format_with_location(db, "Failed to get declaration line for parameter")
            .into());
    };

    Ok(Variable::new(db, name, ty, file, line, entry))
}

fn resolve_variable_entry<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<Variable<'db>> {
    let Some(name) = entry.name(db) else {
        return Err(entry
            .format_with_location(db, "Failed to get variable name")
            .into());
    };

    let Some(type_entry) = entry.get_unit_ref(db, gimli::DW_AT_type)? else {
        return Err(entry
            .format_with_location(db, "Failed to get type for parameter")
            .into());
    };

    let ty = resolve_type_offset(db, type_entry)?;

    let Some(file) = declaration_file(db, entry) else {
        return Err(entry
            .format_with_location(db, "Failed to get declaration file for parameter")
            .into());
    };
    let Some(line) = entry
        .get_attr(db, gimli::DW_AT_decl_line)
        .and_then(|v| v.udata_value())
    else {
        return Err(entry
            .format_with_location(db, "Failed to get declaration line for parameter")
            .into());
    };
    Ok(Variable::new(db, name, ty, file, line, entry))
}
