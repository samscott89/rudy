//! DWARF expression evaluation for location information

use anyhow::Result;

use crate::database::Db;
use crate::dwarf::{Die, loader::DwarfReader};
use crate::file::{Binary, File};
use crate::types::FunctionIndexEntry;

/// Get location expression from a DIE entry
#[salsa::tracked]
pub fn get_location_expr<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    attr: gimli::DwAt,
) -> Option<gimli::Expression<DwarfReader>> {
    let Some(location) = entry.get_attr(db, attr) else {
        db.report_warning(format!(
            "Failed to get location attribute for entry: {}",
            entry.print(db)
        ));
        return None;
    };

    let gimli::AttributeValue::Exprloc(expr) = location else {
        db.report_critical(format!(
            "Location not an expression for entry: {}",
            entry.print(db)
        ));
        return None;
    };

    Some(expr)
}

/// Get function frame base register
fn get_function_frame_base<'db>(
    db: &'db dyn Db,
    function_entry: Die<'db>,
    _data_resolver: &dyn crate::DataResolver,
) -> Result<gimli::Register> {
    let Some(loc_exp) = get_location_expr(db, function_entry, gimli::DW_AT_frame_base) else {
        anyhow::bail!("Failed to get location expression for function");
    };
    // evaluation the expression
    let Some(unit_ref) = function_entry.unit_ref(db) else {
        anyhow::bail!("Failed to get unit ref for function");
    };

    let mut eval = loc_exp.evaluation(unit_ref.encoding());
    let result = eval.evaluate()?;
    let result = loop {
        match result {
            gimli::EvaluationResult::Complete => {
                // evaluation complete -- break the loop
                break eval.result();
            }
            r => {
                todo!("handle incomplete evaluation: {r:?}");
            }
        }
    };

    debug_assert_eq!(result.len(), 1, "got: {result:#?}");
    let result = result[0].clone();
    let gimli::Location::Register { register } = result.location else {
        anyhow::bail!("Expected register location, got: {result:?}");
    };
    Ok(register)
}

/// Resolve data location for a variable using DWARF expressions
pub fn resolve_data_location<'db>(
    db: &'db dyn Db,
    binary: File,
    function: FunctionIndexEntry<'db>,
    variable_entry_id: Die<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Option<u64>> {
    let function_entry = function.die(db);
    let Some(expr) = get_location_expr(db, variable_entry_id, gimli::DW_AT_location) else {
        return Ok(None);
    };

    let Some(unit_ref) = function_entry.unit_ref(db) else {
        return Ok(None);
    };

    // evaluation the expression

    let mut eval = expr.evaluation(unit_ref.encoding());
    let mut result = eval.evaluate()?;
    let result = loop {
        match result {
            gimli::EvaluationResult::Complete => {
                // evaluation complete -- break the loop
                break eval.result();
            }
            gimli::EvaluationResult::RequiresFrameBase => {
                // get the frame base from the enclosing function
                let reg = get_function_frame_base(db, function_entry, data_resolver)?.0;
                let frame_base = data_resolver.get_register(reg as usize)?;
                tracing::debug!("register: {reg} = {frame_base:#x}");

                result = eval.resume_with_frame_base(frame_base)?;
            }
            gimli::EvaluationResult::RequiresRegister { register, .. } => {
                let reg = register.0;
                let reg_value = data_resolver.get_register(reg as usize)?;
                tracing::debug!("register value: {reg} = {reg_value:#x}");
                result = eval.resume_with_register(gimli::Value::Generic(reg_value))?;
            }
            gimli::EvaluationResult::RequiresRelocatedAddress(addr) => {
                // We have an address that is relative to where
                // the data is loaded an need to shift it appropriately
                let cu = function_entry.cu(db);
                let base_addr = crate::index::build_index(db, binary)
                    .data(db)
                    .cu_to_base_addr
                    .get(&cu)
                    .copied();
                let Some(base_addr) = base_addr else {
                    db.report_critical(format!("Failed to get base address"));
                    return Ok(None);
                };
                let relocated_addr = base_addr + addr;

                tracing::debug!("relocated address: {addr:#x} -> {relocated_addr:#x}",);
                result = eval.resume_with_relocated_address(relocated_addr)?;
            }
            r => {
                todo!("handle incomplete evaluation: {r:?}");
            }
        }
    };

    // let mut data_buffer = vec![];
    if let [piece] = &result[..] {
        tracing::debug!("single piece: {piece:#?}");
        match &piece.location {
            gimli::Location::Address { address } => {
                tracing::debug!("address: {address:#x}");
                Ok(Some(*address))
            }
            loc => {
                todo!("handle location: {loc:#?}");
            }
        }
    } else {
        todo!("support multiple pieces: {result:#?}");
    }
}
