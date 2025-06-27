//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use anyhow::{Context, Result, anyhow};
use rudy_db::{DataResolver, DebugInfo, Value};
use rudy_types::{StdLayout, TypeLayout};
use std::cell::RefCell;
use std::sync::Arc;

use crate::protocol::{EventRequest, EventResponseData};
use crate::server::ClientConnection;
use rudy_parserr::Expression;

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
        Vec<rudy_db::Variable>,
        Vec<rudy_db::Variable>,
        Vec<rudy_db::Variable>,
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
            Expression::NumberLiteral(value) => Ok(EvalResult {
                value: value.to_string(),
                type_name: "u64".to_string(),
            }),
            Expression::StringLiteral(value) => Ok(EvalResult {
                value: format!("\"{}\"", value),
                type_name: "String".to_string(),
            }),
            Expression::Parenthesized(inner) => self.evaluate(inner),
            Expression::Deref(_) => Err(anyhow!(
                "Pointer dereferencing not supported - values are automatically dereferenced"
            )),
            Expression::AddressOf { .. } => Err(anyhow!(
                "Address-of operator not supported in debugging context"
            )),
        }
    }

    /// Get the type of an expression without evaluating its value
    pub fn get_expression_type(&mut self, expr: &Expression) -> Result<rudy_types::TypeLayout> {
        let value_ref = self.evaluate_to_ref(expr)?;
        Ok((*value_ref.type_def).clone())
    }

    /// Evaluates an expression to a ValueRef (for intermediate computation)
    fn evaluate_to_ref(&mut self, expr: &Expression) -> Result<ValueRef> {
        match expr {
            Expression::Variable(name) => self.evaluate_variable_to_ref(name),
            Expression::FieldAccess { base, field } => {
                self.evaluate_field_access_to_ref(base, field)
            }
            Expression::Index { base, index } => self.evaluate_index_to_ref(base, index),
            // Literals, deref, and address-of don't have memory locations
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

        let field_info = self
            .debug_info
            .get_field(base_ref.address, &base_ref.type_def, field)?;

        Ok(ValueRef {
            address: field_info
                .address
                .ok_or_else(|| anyhow!("Field has no address"))?,
            type_def: field_info.type_def,
        })
    }

    fn evaluate_index_to_ref(&mut self, base: &Expression, index: &Expression) -> Result<ValueRef> {
        let base_ref = self.evaluate_to_ref(base)?;

        // Check if the base type supports string indexing (HashMap, etc.)
        if self.supports_string_indexing(&base_ref.type_def) {
            let key_string = self.evaluate_to_string(index)?;
            // Use get_index_by_value with string key
            let key_value = Value::Scalar {
                ty: "String".to_string(),
                value: key_string,
            };
            let element_info = self.debug_info.get_index_by_value(
                base_ref.address,
                &base_ref.type_def,
                &key_value,
                &self.conn,
            )?;

            // HashMap values don't have direct memory addresses, so we need special handling
            if element_info.address.is_none() {
                // For HashMap, we can't create a ValueRef since there's no memory address
                // This means we need to handle HashMap indexing at the EvalResult level
                return Err(anyhow!(
                    "HashMap indexing requires special handling - use evaluate_index instead"
                ));
            }

            Ok(ValueRef {
                address: element_info.address.unwrap(),
                type_def: element_info.type_def,
            })
        } else {
            // Default to integer indexing
            let index_int = self.evaluate_to_int(index)?;
            let element_info = self.debug_info.get_index_by_int(
                base_ref.address,
                &base_ref.type_def,
                index_int,
                &self.conn,
            )?;

            Ok(ValueRef {
                address: element_info
                    .address
                    .ok_or_else(|| anyhow!("Element has no address"))?,
                type_def: element_info.type_def,
            })
        }
    }

    /// Evaluate an expression to an integer value
    fn evaluate_to_int(&mut self, expr: &Expression) -> Result<u64> {
        match expr {
            Expression::NumberLiteral(val) => Ok(*val),
            Expression::Variable(name) => {
                let var_ref = self.evaluate_variable_to_ref(name)?;
                self.read_integer_from_memory(&var_ref)
            }
            Expression::FieldAccess { base, field } => {
                let field_ref = self.evaluate_field_access_to_ref(base, field)?;
                self.read_integer_from_memory(&field_ref)
            }
            Expression::Index { base, index } => {
                let element_ref = self.evaluate_index_to_ref(base, index)?;
                self.read_integer_from_memory(&element_ref)
            }
            Expression::Parenthesized(inner) => self.evaluate_to_int(inner),
            _ => Err(anyhow!("Cannot evaluate expression to integer: {:?}", expr)),
        }
    }

    /// Evaluate an expression to a string value  
    fn evaluate_to_string(&mut self, expr: &Expression) -> Result<String> {
        match expr {
            Expression::StringLiteral(value) => Ok(value.clone()),
            Expression::Variable(name) => {
                let var_ref = self.evaluate_variable_to_ref(name)?;
                self.read_string_from_memory(&var_ref)
            }
            Expression::FieldAccess { base, field } => {
                let field_ref = self.evaluate_field_access_to_ref(base, field)?;
                self.read_string_from_memory(&field_ref)
            }
            Expression::Index { base, index } => {
                let element_ref = self.evaluate_index_to_ref(base, index)?;
                self.read_string_from_memory(&element_ref)
            }
            Expression::Parenthesized(inner) => self.evaluate_to_string(inner),
            // String literals would need to be added to the parser
            _ => Err(anyhow!("Cannot evaluate expression to string: {:?}", expr)),
        }
    }

    /// Check if a type supports string-based indexing (HashMap, etc.)
    fn supports_string_indexing(&self, type_def: &TypeLayout) -> bool {
        match type_def {
            TypeLayout::Std(std_def) => {
                matches!(std_def, StdLayout::Map(_))
            }
            _ => false,
        }
    }

    /// Read an integer value from memory using a ValueRef
    fn read_integer_from_memory(&self, value_ref: &ValueRef) -> Result<u64> {
        let value =
            self.debug_info
                .address_to_value(value_ref.address, &value_ref.type_def, &self.conn)?;
        match value {
            Value::Scalar { value, .. } => {
                // Try to parse as different number formats
                if let Ok(num) = value.parse::<u64>() {
                    Ok(num)
                } else if value.starts_with("0x") {
                    u64::from_str_radix(&value[2..], 16)
                        .with_context(|| format!("Failed to parse hex value: {}", value))
                } else {
                    Err(anyhow!("Could not parse integer value: {}", value))
                }
            }
            _ => Err(anyhow!("Expected scalar integer value, got: {:?}", value)),
        }
    }

    /// Read a string value from memory using a ValueRef
    fn read_string_from_memory(&self, value_ref: &ValueRef) -> Result<String> {
        let value =
            self.debug_info
                .address_to_value(value_ref.address, &value_ref.type_def, &self.conn)?;
        match value {
            Value::Scalar { value, .. } => {
                // For strings, the formatted value should be the string content
                // We might need to strip quotes depending on formatting
                let trimmed = value.trim_matches('"');
                Ok(trimmed.to_string())
            }
            _ => Err(anyhow!("Expected scalar string value, got: {:?}", value)),
        }
    }

    fn evaluate_index(&mut self, base: &Expression, index: &Expression) -> Result<EvalResult> {
        let base_ref = self.evaluate_to_ref(base)?;

        // Check if the base type supports string indexing (HashMap, etc.)
        if self.supports_string_indexing(&base_ref.type_def) {
            let key_string = self.evaluate_to_string(index)?;
            tracing::debug!(
                "Evaluating index with string key: {} for base type: {}",
                key_string,
                base_ref.type_def.display_name()
            );
            // Use get_index_by_value with string key
            let key_value = Value::Scalar {
                ty: "String".to_string(),
                value: key_string,
            };
            let element_info = self.debug_info.get_index_by_value(
                base_ref.address,
                &base_ref.type_def,
                &key_value,
                &self.conn,
            )?;

            // HashMap values don't have direct memory addresses, so we need special handling
            if element_info.address.is_none() {
                // For HashMap, we need to read the value directly since it was already resolved
                let value = self.debug_info.read_variable(&element_info, &self.conn)?;
                return Ok(EvalResult {
                    value: format_value(&value),
                    type_name: element_info.type_def.display_name(),
                });
            }

            // If we do have an address, create a ValueRef and convert normally
            let element_ref = ValueRef {
                address: element_info.address.unwrap(),
                type_def: element_info.type_def,
            };
            self.value_ref_to_result(&element_ref)
        } else {
            // Default to integer indexing
            let index_int = self.evaluate_to_int(index)?;
            let element_info = self.debug_info.get_index_by_int(
                base_ref.address,
                &base_ref.type_def,
                index_int,
                &self.conn,
            )?;

            let element_ref = ValueRef {
                address: element_info
                    .address
                    .ok_or_else(|| anyhow!("Element has no address"))?,
                type_def: element_info.type_def,
            };
            self.value_ref_to_result(&element_ref)
        }
    }
}

/// Reference to a typed value in memory (used for intermediate evaluation)
#[derive(Debug, Clone)]
struct ValueRef {
    /// Memory address where the value is stored
    address: u64,
    /// Full type definition for the value
    type_def: Arc<TypeLayout>,
}

/// Final result of evaluating an expression (for display/serialization)
#[derive(Debug, serde::Serialize, Clone)]
pub struct EvalResult {
    /// The evaluated value (formatted for display)
    pub value: String,
    /// The type of the value
    #[serde(rename = "type")]
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
        Value::Tuple { ty, entries } => {
            let entries_str: Vec<String> = entries.iter().map(format_value).collect();
            format!("{ty} (\n{}\n)", indent(&entries_str.join(",\n"), 1))
        }
    }
}
