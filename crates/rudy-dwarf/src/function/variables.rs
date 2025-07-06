//! Variable resolution from DWARF debugging information

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
    types::DieTypeDefinition,
    visitor::{self, DieVisitor},
    Die, DwarfDb,
};

type Result<T> = std::result::Result<T, crate::Error>;

/// Tracked variable information
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct Variable {
    pub name: Option<String>,
    pub ty: DieTypeDefinition,
    pub location: Option<SourceLocation>,
    /// The DIE that this variable was parsed from
    pub origin: Die,
}

impl Variable {
    pub fn new(
        name: Option<String>,
        ty: DieTypeDefinition,
        location: Option<SourceLocation>,
        origin: Die,
    ) -> Self {
        Self {
            name,
            ty,
            location,
            origin,
        }
    }
}

/// Resolved variables for a function
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct ResolvedVariables {
    pub params: Vec<Variable>,
    pub locals: Vec<Variable>,
}

struct VariableVisitor {
    params: Vec<Variable>,
    locals: Vec<Variable>,
}

impl<'db> DieVisitor<'db> for VariableVisitor {
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
) -> Result<ResolvedVariables> {
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
    Ok(ResolvedVariables {
        params: visitor.params,
        locals: visitor.locals,
    })
}

pub fn variable() -> impl Parser<Variable> {
    all((
        optional_attr(gimli::DW_AT_name),
        entry_type().then(resolve_type_shallow()),
        from_fn(position),
    ))
    .map_with_entry(|_db, entry, (name, ty, position)| Variable::new(name, ty, position, entry))
}
