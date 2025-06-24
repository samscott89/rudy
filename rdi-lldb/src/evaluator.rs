//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use anyhow::{Result, anyhow};
use rust_debuginfo::{DataResolver, DebugInfo, Value};
use std::cell::RefCell;

use crate::expression::Expression;
use crate::protocol::{EventRequest, EventResponseData};
use crate::server::ClientConnection;

/// Remote client access for synchronous data fetching
pub struct RemoteDataAccess<'conn> {
    conn: RefCell<&'conn mut ClientConnection>,
}

impl<'conn> RemoteDataAccess<'conn> {
    pub fn new(conn: &'conn mut ClientConnection) -> Self {
        Self {
            conn: RefCell::new(conn),
        }
    }

    #[allow(dead_code)]
    pub fn read_register(&mut self, name: &str) -> Result<u64> {
        let event = EventRequest::ReadRegister {
            name: name.to_string(),
        };
        let response = self.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::RegisterData { value } => Ok(value),
            EventResponseData::Error { message } => {
                Err(anyhow!("Register read failed: {}", message))
            }
            _ => Err(anyhow!("Unexpected response type for ReadRegister")),
        }
    }
}

impl<'conn> DataResolver for RemoteDataAccess<'conn> {
    fn base_address(&self) -> u64 {
        // Get the base load address where the binary is loaded in memory
        if let Ok(EventResponseData::BaseAddress { address }) = self
            .conn
            .borrow_mut()
            .send_event_request(EventRequest::GetBaseAddress)
        {
            address
        } else {
            0 // Fallback if we can't get base address
        }
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        let event = EventRequest::ReadMemory { address, size };
        let response: EventResponseData = self.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::MemoryData { data } => Ok(data),
            EventResponseData::Error { message } => Err(anyhow!("{message}")),
            _ => Err(anyhow!("Unexpected response type for ReadMemory")),
        }
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        // This method is supposed to return all available registers
        // For now, we'll return an error since we don't have a way to get all registers
        // In practice, DWARF expressions typically use get_register(idx) directly
        Err(anyhow!(
            "get_registers() not implemented - use get_register(idx) instead"
        ))
    }

    fn get_register(&self, idx: usize) -> Result<u64> {
        // Read a specific register by index using the new protocol event
        let event = EventRequest::ReadRegisterByIndex { index: idx };
        let response = self.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::RegisterData { value } => Ok(value),
            EventResponseData::Error { message } => Err(anyhow!(
                "Register read failed for index {}: {}",
                idx,
                message
            )),
            _ => Err(anyhow!("Unexpected response type for ReadRegisterByIndex")),
        }
    }

    fn read_address(&self, address: u64) -> Result<u64> {
        // Read 8 bytes as u64 (assuming 64-bit addresses)
        let bytes = self.read_memory(address, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
            anyhow!("Expected 8 bytes for address read")
        })?))
    }
}

pub struct EvalContext<'a> {
    /// Debug information for the current binary
    debug_info: DebugInfo<'a>,
    conn: RemoteDataAccess<'a>,
}

impl<'a> EvalContext<'a> {
    pub fn new(debug_info: DebugInfo<'a>, conn: &'a mut ClientConnection) -> Self {
        Self {
            debug_info,
            conn: RemoteDataAccess::new(conn),
        }
    }

    /// Resolve variables at the current program counter
    #[allow(dead_code)]
    fn resolve_variables_at_address(
        &self,
        address: u64,
        resolver: &dyn DataResolver,
    ) -> Result<(
        Vec<rust_debuginfo::Variable>,
        Vec<rust_debuginfo::Variable>,
        Vec<rust_debuginfo::Variable>,
    )> {
        self.debug_info
            .resolve_variables_at_address(address, resolver)
    }

    /// Evaluates an expression, potentially generating events for the client
    pub fn evaluate(&self, expr: &Expression) -> Result<EvalResult> {
        match expr {
            Expression::Variable(name) => self.evaluate_variable(name),
        }
    }

    fn evaluate_variable(&self, name: &str) -> Result<EvalResult> {
        let EventResponseData::FrameInfo { pc, .. } = self
            .conn
            .conn
            .borrow_mut()
            .send_event_request(EventRequest::GetFrameInfo)?
        else {
            return Err(anyhow!("unexpected response type for GetFrameInfo"));
        };
        // Try to resolve variables at the current PC
        let (params, locals, _globals) = self
            .debug_info
            .resolve_variables_at_address(pc, &self.conn)?;
        // Search for the variable by name in parameters and locals
        let mut all_vars = params.iter().chain(locals.iter());

        if let Some(variable) = all_vars.find(|var| var.name == name) {
            // Found the variable! Extract type and value information
            let type_name = variable
                .ty
                .as_ref()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            let value_str = match &variable.value {
                Some(value) => format_value(value),
                None => "<value not available>".to_string(),
            };

            Ok(EvalResult {
                value: value_str,
                type_name,
            })
        } else {
            // Variable not found at this location
            Ok(EvalResult {
                value: "<not found>".to_string(),
                type_name: "Unknown".to_string(),
            })
        }
    }
}

/// Result of evaluating an expression
#[derive(Debug, serde::Serialize)]
pub struct EvalResult {
    /// The evaluated value (formatted for display)
    pub value: String,
    /// The type of the value
    pub type_name: String,
}

fn indent(s: &str, level: usize) -> String {
    let indent = " ".repeat(level * 2);
    s.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format a Value for display
fn format_value(value: &Value) -> String {
    match value {
        Value::Scalar { ty: _, value } => value.clone(),
        Value::Array { ty: _, items } => {
            if items.len() <= 10 {
                let items_str: Vec<String> = items.iter().map(format_value).collect();
                format!("[\n{}\n]", indent(&items_str.join(",\n"), 1))
            } else {
                format!("[{} items]", items.len())
            }
        }
        Value::Struct { ty, fields } => {
            let fields_str: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_value(v)))
                .collect();
            format!("{ty} {{\n{}\n}}", indent(&fields_str.join(",\n"), 1))
        }
        Value::Map { entries, .. } => {
            if entries.len() <= 10 {
                let fields_str: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", format_value(k), format_value(v)))
                    .collect();
                format!("{{\n{}\n}}", indent(&fields_str.join(",\n"), 1))
            } else {
                format!("{{ {} entries }}", entries.len())
            }
        }
    }
}

// Future implementations:
// - evaluate_field_access
// - evaluate_index
// - evaluate_deref
// - evaluate_method_call
