//! Variable resolution from DWARF debugging information

use rudy_types::Layout;

use crate::{
    die::position,
    file::SourceLocation,
    function::FunctionIndexEntry,
    parser::{
        combinators::all,
        from_fn,
        primitives::{entry_type, optional_attr, resolve_type_shallow},
        Parser,
    },
    visitor::{self, DieVisitor},
    Die, DwarfDb,
};

type Result<T> = std::result::Result<T, crate::Error>;

/// Tracked variable information
#[salsa::tracked(debug)]
pub struct Variable<'db> {
    #[returns(ref)]
    pub name: Option<String>,
    #[returns(ref)]
    pub ty: Layout,
    pub location: Option<SourceLocation<'db>>,
    /// The DIE that this variable was parsed from
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
    fn visit_variable<'a>(
        walker: &mut crate::visitor::DieWalker<'a, 'db, Self>,
        node: crate::visitor::VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        let entry = walker.get_die(node.die);
        let db = walker.db;
        tracing::debug!("variable: {}", entry.print(db));

        match variable().parse(db, entry) {
            Ok(var) => {
                walker.visitor.locals.push(var);
            }
            Err(e) => {
                tracing::warn!("Failed to resolve variable: {e} in {}", entry.location(db));
            }
        }
        Ok(())
    }

    fn visit_parameter<'a>(
        walker: &mut crate::visitor::DieWalker<'a, 'db, Self>,
        node: crate::visitor::VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        let entry = walker.get_die(node.die);
        let db = walker.db;
        tracing::debug!("param: {}", entry.print(db));
        match variable().parse(db, entry) {
            Ok(var) => {
                walker.visitor.params.push(var);
            }
            Err(e) => {
                tracing::warn!("Failed to resolve parameter: {e} in {}", entry.location(db));
            }
        }
        Ok(())
    }
}

/// Resolve all variables in a function
///
/// TODO(Sam): I think I should move globals outside of this
/// since those generally happen at the top level and don't
/// necessarily appear in this function's DIE.
#[salsa::tracked]
pub fn resolve_function_variables<'db>(
    db: &'db dyn DwarfDb,
    fie: FunctionIndexEntry<'db>,
) -> Result<ResolvedVariables<'db>> {
    let data = fie.data(db);
    let die = data.specification_die.unwrap_or(data.declaration_die);

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

pub fn variable<'db>() -> impl Parser<'db, Variable<'db>> {
    all((
        optional_attr(gimli::DW_AT_name),
        entry_type().then(resolve_type_shallow()),
        from_fn(position),
    ))
    .map_with_db_and_entry(|db, entry, (name, ty, position)| {
        Variable::new(db, name, ty, position, entry)
    })
}
