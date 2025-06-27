//! Variable resolution from DWARF debugging information

use crate::database::Db;
use crate::dwarf::die::declaration_file;
use crate::dwarf::visitor::{self, DieVisitor};
use crate::dwarf::{Die, resolution::types::resolve_type_offset};
use crate::file::SourceFile;
use rust_types::{TypeLayout, UnitLayout};

type Result<T> = std::result::Result<T, super::Error>;

/// Tracked variable information
#[salsa::tracked]
pub struct Variable<'db> {
    #[returns(ref)]
    pub name: String,
    #[returns(ref)]
    pub ty: TypeLayout,
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

struct VariableVisitor<'db> {
    params: Vec<Variable<'db>>,
    locals: Vec<Variable<'db>>,
}

impl<'db> DieVisitor<'db> for VariableVisitor<'db> {
    fn visit_function<'a>(
        walker: &mut visitor::DieWalker<'a, 'db, Self>,
        entry: crate::dwarf::loader::RawDie<'a>,
        _unit_ref: crate::dwarf::unit::UnitRef<'a>,
    ) {
        tracing::debug!("function: {}", walker.get_die(entry).print(walker.db));
        walker.walk_children();
    }

    fn visit_lexical_block<'a>(
        walker: &mut crate::dwarf::visitor::DieWalker<'a, 'db, Self>,
        _entry: crate::dwarf::loader::RawDie<'a>,
        _unit_ref: crate::dwarf::unit::UnitRef<'a>,
    ) {
        tracing::debug!("lexical block: {}", walker.get_die(_entry).print(walker.db));
        walker.walk_children();
    }

    fn visit_variable<'a>(
        walker: &mut crate::dwarf::visitor::DieWalker<'a, 'db, Self>,
        entry: crate::dwarf::loader::RawDie<'a>,
        _unit_ref: crate::dwarf::unit::UnitRef<'a>,
    ) {
        let entry = walker.get_die(entry);
        let db = walker.db;
        tracing::debug!("variable: {}", entry.print(db));

        match resolve_variable_entry(db, entry) {
            Ok(var) => {
                walker.visitor.locals.push(var);
            }
            Err(e) => {
                db.report_warning(
                    entry.format_with_location(db, format!("Failed to resolve variable: {e}")),
                );
            }
        }
    }

    fn visit_parameter<'a>(
        walker: &mut crate::dwarf::visitor::DieWalker<'a, 'db, Self>,
        entry: crate::dwarf::loader::RawDie<'a>,
        _unit_ref: crate::dwarf::unit::UnitRef<'a>,
    ) {
        let entry = walker.get_die(entry);
        let db = walker.db;
        tracing::debug!("param: {}", entry.print(db));
        if entry.name(db).is_err() {
            // we sometimes encounter anonymous parameters
            // which we'll just push a dummy value
            let file =
                match declaration_file(db, entry) {
                    Ok(file) => file,
                    Err(e) => {
                        db.report_warning(entry.format_with_location(
                            db,
                            format!("Failed to get declaration file: {e}"),
                        ));
                        return;
                    }
                };

            walker.visitor.params.push(Variable::new(
                db,
                format!("__{}", walker.visitor.params.len()),
                TypeLayout::Primitive(rust_types::PrimitiveLayout::Unit(UnitLayout)),
                file,
                0,
                entry,
            ));
            return;
        }
        match resolve_variable_entry(db, entry) {
            Ok(var) => {
                walker.visitor.params.push(var);
            }
            Err(e) => {
                db.report_warning(
                    entry.format_with_location(db, format!("Failed to resolve parameter: {e}")),
                );
            }
        }
    }
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

    let mut visitor = VariableVisitor {
        params: vec![],
        locals: vec![],
    };
    visitor::walk_die(db, die, &mut visitor)?;
    Ok(ResolvedVariables::new(db, visitor.params, visitor.locals))
}

fn resolve_variable_entry<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<Variable<'db>> {
    let name = entry.name(db)?;
    let type_entry = entry.get_referenced_entry(db, gimli::DW_AT_type)?;
    let ty = resolve_type_offset(db, type_entry)?;
    let file = declaration_file(db, entry)?;
    let line = entry.udata_attr(db, gimli::DW_AT_decl_line)?;
    Ok(Variable::new(db, name, ty, file, line as u64, entry))
}
