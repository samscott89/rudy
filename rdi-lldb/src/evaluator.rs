//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use anyhow::{Result, anyhow};
use rust_debuginfo::{DataResolver, DebugInfo};
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
        todo!()
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        let event = EventRequest::ReadMemory { address, size };
        let response: EventResponseData = self.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::MemoryData { data } => Ok(data),
            EventResponseData::Error { message } => Err(anyhow!("Memory read failed: {}", message)),
            _ => Err(anyhow!("Unexpected response type for ReadMemory")),
        }
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        // Return the basic register set we know about
        //
        todo!()
    }

    fn get_register(&self, idx: usize) -> Result<u64> {
        // match idx {
        //     0 => Ok(self.context.pc), // PC
        //     1 => Ok(self.context.sp), // SP
        //     2 => Ok(self.context.fp), // FP
        //     _ => Err(anyhow!("Register index {} not available", idx)),
        // }
        todo!()
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

            let (value_str, pretty_str) = match &variable.value {
                Some(value) => {
                    let value_display = format_value(value);
                    let pretty_display = format!("{} = {}", name, value_display);
                    (value_display, pretty_display)
                }
                None => {
                    // Variable found but no value available
                    let no_value = "<value not available>".to_string();
                    let pretty = format!("{} = {}", name, no_value);
                    (no_value, pretty)
                }
            };

            Ok(EvalResult {
                value: value_str,
                type_name,
                pretty: pretty_str,
            })
        } else {
            // Variable not found at this location
            Ok(EvalResult {
                value: "<not found>".to_string(),
                type_name: "Unknown".to_string(),
                pretty: format!("{} = <variable not found in current scope>", name),
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
    /// Pretty-printed representation
    pub pretty: String,
}

/// Format a Value for display
fn format_value(value: &rust_debuginfo::Value) -> String {
    match value {
        rust_debuginfo::Value::Scalar { ty: _, value } => value.clone(),
        rust_debuginfo::Value::Array { ty: _, items } => {
            if items.len() <= 3 {
                let items_str: Vec<String> = items.iter().map(format_value).collect();
                format!("[{}]", items_str.join(", "))
            } else {
                format!("[{} items]", items.len())
            }
        }
        rust_debuginfo::Value::Struct { ty: _, fields } => {
            if fields.len() <= 2 {
                let fields_str: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, format_value(v)))
                    .collect();
                format!("{{ {} }}", fields_str.join(", "))
            } else {
                format!("{{ {} fields }}", fields.len())
            }
        }
    }
}

// Future implementations:
// - evaluate_field_access
// - evaluate_index
// - evaluate_deref
// - evaluate_method_call
