//! Variable resolution from DWARF debugging information

use crate::data::Def;
use crate::database::Db;
use crate::dwarf::die::declaration_file;
use crate::dwarf::{DieEntryId, resolution::types::resolve_type_offset};
use crate::file::SourceFile;
use crate::types::FunctionIndexEntry;

/// Tracked variable information
#[salsa::tracked]
pub struct Variable<'db> {
    #[return_ref]
    pub name: String,
    #[return_ref]
    pub ty: Def<'db>,
    pub file: SourceFile<'db>,
    pub line: u64,
    pub origin: DieEntryId<'db>,
}

/// Resolved variables for a function
#[salsa::tracked]
pub struct ResolvedVariables<'db> {
    pub params: Vec<Variable<'db>>,
    pub locals: Vec<Variable<'db>>,
    // Globals have a static address already
    pub globals: Vec<(Variable<'db>, u64)>,
}

/// Resolve all variables in a function
#[salsa::tracked]
pub fn resolve_function_variables<'db>(
    db: &'db dyn Db,
    function: FunctionIndexEntry<'db>,
) -> ResolvedVariables<'db> {
    let die = function.die(db);
    tracing::debug!(
        "resolving function variables for `{}` in {}",
        die.name(db).unwrap(),
        die.cu(db).file(db).full_path(db)
    );

    let mut params = vec![];
    let mut locals = vec![];
    let mut globals = vec![];

    // get the file this is declared in
    let Some(function_decl_file) = declaration_file(db, die) else {
        db.report_critical(format!("Failed to get file for function"));
        return ResolvedVariables::new(db, params, locals, globals);
    };

    let index = crate::index::index(db).data(db);

    for (global_symbol, symbol_index) in &index.symbol_name_to_die {
        let symbol_entry = symbol_index.die(db);
        if declaration_file(db, symbol_entry) == Some(function_decl_file) {
            tracing::debug!(
                "global symbol in scope: {}:{}",
                global_symbol.as_path(db),
                symbol_entry.print(db)
            );
            let Some(var) = resolve_variable_entry(db, symbol_entry) else {
                db.report_critical(format!("Failed to parse parameter entry"));
                continue;
            };
            globals.push((var, symbol_index.address(db)));
        }
    }

    // recurse function to find params + locals
    for child in die.children(db) {
        match child.tag(db) {
            gimli::DW_TAG_formal_parameter => {
                tracing::debug!("parameter: {}", child.print(db));
                let Some(param) = resolve_function_parameter_entry(db, child) else {
                    db.report_critical(format!("Failed to parse parameter entry"));
                    continue;
                };
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
                        let Some(var) = resolve_variable_entry(db, grandchild) else {
                            db.report_critical(format!("Failed to parse parameter entry"));
                            continue;
                        };
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
    ResolvedVariables::new(db, params, locals, globals)
}

fn resolve_function_parameter_entry<'db>(
    db: &'db dyn Db,
    entry: DieEntryId<'db>,
) -> Option<Variable<'db>> {
    let name = entry.name(db)?;

    let type_offset_val = entry.get_attr(db, gimli::DW_AT_type)?;
    let gimli::AttributeValue::UnitRef(type_offset) = type_offset_val else {
        db.report_critical(format!("Unexpected type offset: {type_offset_val:?}"));
        return None;
    };

    let type_entry = DieEntryId::new(db, entry.file(db), entry.cu_offset(db), type_offset);

    let ty = resolve_type_offset(db, type_entry)?;

    let file = declaration_file(db, entry)?;
    let line = entry.get_attr(db, gimli::DW_AT_decl_line)?.udata_value()?;

    Some(Variable::new(db, name, ty, file, line, entry))
}

fn resolve_variable_entry<'db>(db: &'db dyn Db, entry: DieEntryId<'db>) -> Option<Variable<'db>> {
    let name = entry.name(db)?;

    let type_offset_val = entry.get_attr(db, gimli::DW_AT_type)?;
    let gimli::AttributeValue::UnitRef(type_offset) = type_offset_val else {
        db.report_critical(format!("Unexpected type offset: {type_offset_val:?}"));
        return None;
    };

    let type_entry = DieEntryId::new(db, entry.file(db), entry.cu_offset(db), type_offset);

    let ty = resolve_type_offset(db, type_entry)?;

    let decl_file_attr = entry.get_attr(db, gimli::DW_AT_decl_file);
    let Some(gimli::AttributeValue::FileIndex(file_idx)) = decl_file_attr else {
        db.report_critical(format!(
            "Failed to get decl_file attribute, got: {decl_file_attr:?}"
        ));
        return None;
    };

    // get the file from the line program
    let unit_ref = entry.unit_ref(db)?;
    let Some(line_program) = unit_ref.line_program.clone() else {
        db.report_critical(format!("Failed to parse line program"));
        return None;
    };
    let header = line_program.header();
    let Some(file) = header.file(file_idx) else {
        db.report_critical(format!(
            "Failed to parse file index: {:#?}",
            header.file_names()
        ));
        return None;
    };
    let Some(path) = crate::dwarf::utils::file_entry_to_path(file, &unit_ref) else {
        db.report_critical(format!("Failed to convert file entry to path"));
        return None;
    };
    let file = SourceFile::new(db, path);
    let line = entry.get_attr(db, gimli::DW_AT_decl_line)?.udata_value()?;

    Some(Variable::new(db, name, ty, file, line, entry))
}
