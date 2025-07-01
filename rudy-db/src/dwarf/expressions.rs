//! DWARF expression evaluation for location information

use anyhow::{Context, Result};

use crate::database::Db;
use crate::dwarf::{Die, loader::DwarfReader};

/// Get location expression from a DIE entry
#[salsa::tracked]
pub fn get_location_expr<'db>(
    db: &'db dyn Db,
    entry: Die<'db>,
    attr: gimli::DwAt,
) -> Option<gimli::Expression<DwarfReader>> {
    let location = match entry.get_attr(db, attr) {
        Ok(l) => l,
        Err(e) => {
            db.report_warning(format!(
                "Failed to get location attribute for entry: {}: {e}",
                entry.print(db)
            ));
            return None;
        }
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
    data_resolver: &dyn crate::DataResolver,
) -> Result<u64> {
    let Some(loc_exp) = get_location_expr(db, function_entry, gimli::DW_AT_frame_base) else {
        anyhow::bail!("Failed to get location expression for function");
    };
    // evaluation the expression
    let unit_ref = function_entry.unit_ref(db)?;

    let mut eval = loc_exp.evaluation(unit_ref.encoding());
    let mut result = eval.evaluate()?;
    let result = loop {
        match result {
            gimli::EvaluationResult::Complete => {
                // evaluation complete -- break the loop
                break eval.result();
            }
            gimli::EvaluationResult::RequiresCallFrameCfa => {
                let sp = data_resolver.get_stack_pointer()?;
                result = eval.resume_with_call_frame_cfa(sp).with_context(|| {
                    format!("Failed to resume evaluation with call frame CFA with sp: {sp:#x}")
                })?;
            }
            r => {
                todo!("handle incomplete evaluation: {r:?}");
            }
        }
    };

    debug_assert_eq!(result.len(), 1, "got: {result:#?}");

    let result = result[0].clone();

    match result.location {
        // We expect the location to be an address
        gimli::Location::Address { address } => {
            tracing::debug!("frame base address: {address:#x}");
            Ok(address)
        }
        gimli::Location::Register { register, .. } => {
            let reg_value = data_resolver.get_register(register.0 as usize)?;
            tracing::debug!("frame base register value: {reg_value:#x}");
            Ok(reg_value)
        }
        loc => Err(anyhow::anyhow!(
            "Unexpected location type for frame base: {loc:?}"
        )),
    }
}

/// Resolve data location for a variable using DWARF expressions
pub fn resolve_data_location<'db>(
    db: &'db dyn Db,
    function: Die<'db>,
    base_address: u64,
    variable_entry_id: Die<'db>,
    data_resolver: &dyn crate::DataResolver,
) -> Result<Option<u64>> {
    let Some(expr) = get_location_expr(db, variable_entry_id, gimli::DW_AT_location) else {
        return Ok(None);
    };

    let unit_ref = function.unit_ref(db)?;

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
                let frame_base = get_function_frame_base(db, function, data_resolver)?;
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
                let relocated_addr = base_address + addr;

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
