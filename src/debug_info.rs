use anyhow::{Context, Result};
use std::fmt;

use crate::{
    ResolvedAddress, ResolvedLocation,
    data::TypeDef,
    database::{Db, Diagnostic, handle_diagnostics},
    dwarf::{self, resolve_function_variables},
    file::Binary,
    index,
    outputs::ResolvedFunction,
    query::{
        find_closest_match, lookup_address, lookup_closest_function, lookup_position, test_get_def,
    },
    types::{Address, FunctionIndexEntry, NameId, Position},
};

pub struct DebugInfo {
    binary: Binary,
    db: crate::database::DebugDatabaseImpl,
}

impl fmt::Debug for DebugInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugInfo").finish()
    }
}

impl DebugInfo {
    pub fn new(binary_path: &str) -> Result<Self> {
        let mut db = crate::database::DebugDatabaseImpl::new()?;
        let binary = db
            .analyze_file(binary_path)
            .with_context(|| format!("Failed to analyze binary file: {binary_path}"))?;

        let pb = Self { db, binary };
        Ok(pb)
    }

    pub fn address_to_line(&self, address: u64) -> Option<ResolvedLocation> {
        self.resolve_address_to_location(address).unwrap()
    }

    // pub fn resolve_function(&self, name: &str) -> Option<ResolvedAddress> {
    //     let f = self.lookup_function(name).unwrap()?;
    //     let address = f.relative_body_address(&self.db);
    //     Some(ResolvedAddress { address })
    // }

    // pub fn get_source_lines(&self, _address: u64) -> Vec<String> {
    //     todo!()
    // }

    // pub fn resolve_variables_at_address(
    //     &self,
    //     address: u64,
    //     data_resolver: &dyn DataResolver,
    // ) -> (Vec<Variable>, Vec<Variable>, Vec<Variable>) {
    //     let (locals, params, globals) = self
    //         .db
    //         .resolve_variables_at_address(address, data_resolver)
    //         .unwrap();
    //     (locals, params, globals)
    // }

    // pub fn test_get_shape(&self) -> TypeDef<'_> {
    //     self.db.test_get_shape().unwrap()
    // }

    pub fn resolve_function(&self, function: &str) -> Result<Option<ResolvedFunction>> {
        let mut split: Vec<String> = function.split("::").map(|s| s.to_owned()).collect();
        let name = split
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Invalid empty function name: {function}"))?;
        let module_prefix = split;
        let function_name = NameId::new(&self.db, module_prefix, name);

        let Some((_, entry)) = find_closest_match(&self.db, self.binary, function_name) else {
            tracing::debug!("no function found for {function}");
            return Ok(None);
        };
        let diagnostics: Vec<&Diagnostic> =
            find_closest_match::accumulated(&self.db, self.binary, function_name);
        handle_diagnostics(&diagnostics)?;

        let Some(f) = dwarf::resolve_function(&self.db, entry) else {
            return Ok(None);
        };
        let diagnostics: Vec<&Diagnostic> = dwarf::resolve_function::accumulated(&self.db, entry);
        handle_diagnostics(&diagnostics)?;

        let die = entry.die(&self.db);
        let index = index::build_index(&self.db, self.binary);
        let base_address = index
            .data(&self.db)
            .cu_to_base_addr
            .get(&die.cu(&self.db))
            .copied()
            .unwrap_or(0);
        let address = f.relative_body_address(&self.db);

        let params = resolve_function_variables(&self.db, self.binary, entry);
        let diagnostics: Vec<&Diagnostic> =
            dwarf::resolve_function_variables::accumulated(&self.db, self.binary, entry);
        handle_diagnostics(&diagnostics)?;

        Ok(Some(ResolvedFunction {
            name: f.name(&self.db).to_string(),
            address: base_address + address,
            params: params
                .params(&self.db)
                .into_iter()
                .map(|var| crate::Variable {
                    name: var.name(&self.db).to_string(),
                    ty: Some(crate::Type {
                        name: var.ty(&self.db).display_name(&self.db),
                    }),
                    value: None,
                })
                .collect(),
        }))
    }

    pub fn resolve_address_to_location(
        &self,
        address: u64,
    ) -> Result<Option<crate::ResolvedLocation>> {
        let address = Address::new(&self.db, address);
        let loc = lookup_address(&self.db, self.binary, address);
        let Some(f) = lookup_closest_function(&self.db, self.binary, address) else {
            tracing::debug!(
                "no function found for address {:#x}",
                address.address(&self.db)
            );
            return Ok(None);
        };
        let Some(function) = dwarf::resolve_function(&self.db, f) else {
            tracing::debug!("failed to resolve function: {f:?}");
            return Ok(None);
        };
        let diagnostics: Vec<&Diagnostic> = dwarf::resolve_function::accumulated(&self.db, f);
        handle_diagnostics(&diagnostics)?;
        tracing::debug!("returned function + loc: {f:?} / {loc:?}");
        Ok(loc.map(|loc| {
            let file = loc.file(&self.db);
            crate::ResolvedLocation {
                function: function.name(&self.db).to_string(),
                file: file.path(&self.db).clone(),
                line: loc.line(&self.db),
            }
        }))
    }

    pub fn resolve_position(
        &self,
        file: &str,
        line: u64,
        column: Option<u64>,
    ) -> Result<Option<crate::ResolvedAddress>> {
        let query = Position::new(&self.db, file.to_string(), line, column);
        let pos = lookup_position(&self.db, self.binary, query);
        Ok(pos.map(|address| crate::ResolvedAddress { address }))
    }

    pub fn resolve_variables_at_address(
        &self,
        address: u64,
        data_resolver: &dyn crate::DataResolver,
    ) -> Result<(
        Vec<crate::Variable>,
        Vec<crate::Variable>,
        Vec<crate::Variable>,
    )> {
        let address = Address::new(&self.db, address);
        let loc = lookup_address(&self.db, self.binary, address);
        let f = lookup_closest_function(&self.db, self.binary, address);
        let diagnostics: Vec<&Diagnostic> =
            lookup_closest_function::accumulated(&self.db, self.binary, address);
        handle_diagnostics(&diagnostics)?;

        let Some(f) = f else {
            tracing::debug!(
                "no function found for address {:#x}",
                address.address(&self.db)
            );
            return Ok(Default::default());
        };

        let vars = dwarf::resolve_function_variables(&self.db, self.binary, f);
        let diagnostics: Vec<&Diagnostic> =
            dwarf::resolve_function_variables::accumulated(&self.db, self.binary, f);
        handle_diagnostics(&diagnostics)?;

        let params = vars
            .params(&self.db)
            .into_iter()
            .map(|param| output_variable(&self.db, self.binary, f, param, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        let locals = vars
            .locals(&self.db)
            .into_iter()
            .filter(|var| {
                // for local variables, we want to make sure the variable
                // is defined before the current location
                if let Some(loc) = loc {
                    loc.line(&self.db) > var.line(&self.db)
                } else {
                    // if we don't have a location, we assume the param is valid
                    true
                }
            })
            .map(|var| output_variable(&self.db, self.binary, f, var, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        let globals = vars
            .globals(&self.db)
            .into_iter()
            .map(|(var, address)| output_global_variable(&self.db, var, address, data_resolver))
            .collect::<Result<Vec<_>>>()?;
        Ok((params, locals, globals))
    }

    pub fn test_get_shape(&self) -> Result<TypeDef<'_>> {
        let test_struct = test_get_def(&self.db, self.binary);
        let diagnostics: Vec<&Diagnostic> = test_get_def::accumulated(&self.db, self.binary);
        handle_diagnostics(&diagnostics)?;
        Ok(test_struct)
    }
}

fn output_variable<'db>(
    db: &'db dyn Db,
    binary: Binary,
    f: FunctionIndexEntry<'db>,
    var: dwarf::Variable<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Variable> {
    let location = dwarf::resolve_data_location(db, binary, f, var.origin(db), data_resolver)?;

    let value = if let Some(addr) = location {
        Some(crate::data::read_from_memory(
            db,
            addr,
            var.ty(db),
            data_resolver,
        )?)
    } else {
        None
    };

    tracing::debug!("variable: {} => {value:?}", var.name(db));

    Ok(crate::Variable {
        name: var.name(db).to_string(),
        ty: Some(crate::Type {
            name: var.ty(db).display_name(db),
        }),
        value,
    })
}

fn output_global_variable<'db>(
    db: &'db dyn Db,
    var: dwarf::Variable<'db>,
    address: u64,
    data_resolver: &dyn crate::DataResolver,
) -> Result<crate::Variable> {
    let value = Some(crate::data::read_from_memory(
        db,
        address,
        var.ty(db),
        data_resolver,
    )?);

    tracing::debug!("variable: {} => {value:?}", var.name(db));

    Ok(crate::Variable {
        name: var.name(db).to_string(),
        ty: Some(crate::Type {
            name: var.ty(db).display_name(db),
        }),
        value,
    })
}
