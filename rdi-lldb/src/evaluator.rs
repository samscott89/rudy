//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use anyhow::{Context, Result, anyhow};
use rust_debuginfo::{DataResolver, DebugInfo, Value};
use std::cell::RefCell;

use crate::protocol::{EventRequest, EventResponseData};
use crate::server::ClientConnection;
use rdi_parser::Expression;

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
    pc: Option<u64>,
}

impl<'a> EvalContext<'a> {
    pub fn new(debug_info: DebugInfo<'a>, conn: &'a mut ClientConnection) -> Self {
        Self {
            debug_info,
            conn: RemoteDataAccess::new(conn),
            pc: None, // Program counter will be set when evaluating expressions
        }
    }

    /// Set the current program counter (used for variable resolution)
    fn get_pc(&mut self) -> Result<u64> {
        if let Some(pc) = self.pc {
            Ok(pc)
        } else {
            // Fetch the current program counter from the client connection
            let EventResponseData::FrameInfo { pc, .. } = self
                .conn
                .conn
                .borrow_mut()
                .send_event_request(EventRequest::GetFrameInfo)?
            else {
                return Err(anyhow!("Unexpected response type for GetFrameInfo"));
            };
            self.pc = Some(pc);
            Ok(pc)
        }
    }

    /// Convert a ValueRef to a final EvalResult by reading and formatting the value
    fn value_ref_to_result(&mut self, value_ref: &ValueRef) -> Result<EvalResult> {
        let value =
            self.debug_info
                .address_to_value(value_ref.address, &value_ref.type_def, &self.conn)?;
        Ok(EvalResult {
            value: format_value(&value),
            type_name: value_ref.type_def.display_name(),
        })
    }

    /// Resolve variables at the current program counter
    #[allow(dead_code)]
    fn resolve_variables_at_address(
        &mut self,
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
    pub fn evaluate(&mut self, expr: &Expression) -> Result<EvalResult> {
        match expr {
            Expression::Variable(name) => self.evaluate_variable(name),
            Expression::FieldAccess { base, field } => self.evaluate_field_access(base, field),
            Expression::Index { base, index } => self.evaluate_index(base, index),
            Expression::Deref(inner) => self.evaluate_deref(inner),
            Expression::AddressOf { mutable: _, expr } => self.evaluate_address_of(expr),
            Expression::Literal(value) => Ok(EvalResult {
                value: value.to_string(),
                type_name: "u64".to_string(),
            }),
            Expression::Parenthesized(inner) => self.evaluate(inner),
        }
    }

    /// Evaluates an expression to a ValueRef (for intermediate computation)
    fn evaluate_to_ref(&mut self, expr: &Expression) -> Result<ValueRef> {
        match expr {
            Expression::Variable(name) => self.evaluate_variable_to_ref(name),
            Expression::FieldAccess { base, field } => {
                self.evaluate_field_access_to_ref(base, field)
            }
            Expression::Index { base, index } => self.evaluate_index_to_ref(base, index),
            Expression::Deref(inner) => self.evaluate_deref_to_ref(inner),
            // Literals and address-of don't have memory locations
            _ => Err(anyhow!(
                "Expression {:?} cannot be evaluated to a memory reference",
                expr
            )),
        }
    }

    fn evaluate_variable(&mut self, name: &str) -> Result<EvalResult> {
        // Try to get a ValueRef, then convert to result
        let value_ref = self.evaluate_variable_to_ref(name)?;
        self.value_ref_to_result(&value_ref)
    }

    fn evaluate_variable_to_ref(&mut self, name: &str) -> Result<ValueRef> {
        let pc = self.get_pc()?;

        let var_info = self
            .debug_info
            .get_variable_at_pc(pc, name, &self.conn)?
            .with_context(|| format!("Failed to resolve variable '{name}'",))?;

        let address = var_info
            .address
            .with_context(|| format!("Variable '{name}' has no memory address"))?;
        Ok(ValueRef {
            address,
            type_def: var_info.type_def,
        })
    }

    fn evaluate_field_access(&mut self, base: &Expression, field: &str) -> Result<EvalResult> {
        // Get the field as a ValueRef, then convert to result
        let field_ref = self.evaluate_field_access_to_ref(base, field)?;
        self.value_ref_to_result(&field_ref)
    }

    fn evaluate_field_access_to_ref(&mut self, base: &Expression, field: &str) -> Result<ValueRef> {
        // First evaluate the base expression to a ValueRef
        let base_ref = self.evaluate_to_ref(base)?;

        // Try to find the field in the type definition
        match &base_ref.type_def {
            rust_debuginfo::TypeDef::Struct(struct_def) => {
                // Look for the field in the struct
                let field_info = struct_def
                    .fields
                    .iter()
                    .find(|f| f.name == field)
                    .with_context(|| {
                        format!("Field '{field}' not found in struct '{}'", struct_def.name)
                    })?;
                let field_addr = base_ref.address + field_info.offset as u64;

                Ok(ValueRef {
                    address: field_addr,
                    type_def: (*field_info.ty).clone(),
                })
            }
            _ => Err(anyhow!(
                "Cannot access field '{}' on non-struct type",
                field
            )),
        }
    }

    fn evaluate_index_to_ref(&mut self, base: &Expression, index: &Expression) -> Result<ValueRef> {
        // TODO: Implement array/slice indexing
        Err(anyhow!("Array indexing not yet implemented"))
    }

    fn evaluate_deref_to_ref(&mut self, expr: &Expression) -> Result<ValueRef> {
        // TODO: Implement pointer dereferencing
        Err(anyhow!("Pointer dereferencing not yet implemented"))
    }

    fn evaluate_index(&mut self, base: &Expression, index: &Expression) -> Result<EvalResult> {
        // Evaluate both base and index
        let base_result = self.evaluate(base)?;
        let index_result = self.evaluate(index)?;

        // For now, return a placeholder
        Ok(EvalResult {
            value: format!("<{}[{}]>", base_result.value, index_result.value),
            type_name: format!("{}[]", base_result.type_name),
        })
    }

    fn evaluate_deref(&mut self, expr: &Expression) -> Result<EvalResult> {
        // Evaluate the inner expression
        let inner_result = self.evaluate(expr)?;

        // For now, return a placeholder
        Ok(EvalResult {
            value: format!("<*{}>", inner_result.value),
            type_name: format!("*{}", inner_result.type_name),
        })
    }

    fn evaluate_address_of(&mut self, expr: &Expression) -> Result<EvalResult> {
        // Evaluate the inner expression
        let inner_result = self.evaluate(expr)?;

        // For now, return a placeholder
        Ok(EvalResult {
            value: format!("<&{}>", inner_result.value),
            type_name: format!("&{}", inner_result.type_name),
        })
    }
}

/// Reference to a typed value in memory (used for intermediate evaluation)
#[derive(Debug, Clone)]
struct ValueRef {
    /// Memory address where the value is stored
    address: u64,
    /// Full type definition for the value
    type_def: rust_debuginfo::TypeDef,
}

/// Final result of evaluating an expression (for display/serialization)
#[derive(Debug, serde::Serialize, Clone)]
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
