//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use std::{cell::RefCell, collections::BTreeMap};

use anyhow::{Context, Result, anyhow};
use itertools::Itertools;
use rudy_db::{
    DataResolver, DebugInfo, TypedPointer, Value, evaluate_synthetic_method, get_synthetic_methods,
};
use rudy_parser::Expression;
use rudy_types::{Layout, Location, StdLayout};

use crate::{
    protocol::{ArgumentType, EventRequest, EventResponseData, MethodArgument, MethodCallResult},
    server::ClientConnection,
};

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

    fn get_stack_pointer(&self) -> Result<u64> {
        Err(anyhow!("get_stack_pointer() not implemented"))
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

    /// Check if a name is a variable in the current LLDB context
    pub fn is_variable(&mut self, name: &str) -> bool {
        let event = EventRequest::GetVariableType {
            name: name.to_string(),
        };

        if let Ok(response) = self.conn.conn.borrow_mut().send_event_request(event) {
            match response {
                EventResponseData::VariableTypeResult { type_name } => type_name.is_some(),
                _ => false,
            }
        } else {
            false
        }
    }

    /// Convert a TypedPointer to a final EvalResult by reading and formatting the value
    fn pointer_to_result(&mut self, pointer: &TypedPointer) -> Result<EvalResult> {
        let mut value = self.debug_info.read_pointer(pointer, &self.conn)?;

        value = self.read_pointer_recursive(&value, 8)?;

        Ok(EvalResult {
            value: format_value(&value),
            type_name: pointer.type_def.display_name(),
        })
    }

    fn read_pointer_recursive(&mut self, value: &Value, max_depth: usize) -> Result<Value> {
        if max_depth == 0 {
            return Ok(value.clone()); // Stop recursion at max depth
        }
        match value {
            Value::Pointer(typed_pointer) => {
                let value = self.debug_info.read_pointer(typed_pointer, &self.conn)?;
                self.read_pointer_recursive(&value, max_depth - 1)
            }
            v @ Value::Scalar { .. } => Ok(v.clone()),
            Value::Array { ty, items } => {
                let items = items
                    .iter()
                    .map(|v| self.read_pointer_recursive(v, max_depth - 1))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Array {
                    ty: ty.clone(),
                    items,
                })
            }
            Value::Struct { ty, fields } => {
                let fields = fields
                    .iter()
                    .map(|(k, v)| {
                        self.read_pointer_recursive(v, max_depth - 1)
                            .map(|v| (k.clone(), v))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
                Ok(Value::Struct {
                    ty: ty.clone(),
                    fields,
                })
            }
            Value::Tuple { ty, entries } => {
                let entries = entries
                    .iter()
                    .map(|v| self.read_pointer_recursive(v, max_depth - 1))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Tuple {
                    ty: ty.clone(),
                    entries,
                })
            }
            Value::Map { ty, entries } => {
                let entries = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = self.read_pointer_recursive(k, max_depth - 1)?;
                        let value = self.read_pointer_recursive(v, max_depth - 1)?;
                        Ok((key, value))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Value::Map {
                    ty: ty.clone(),
                    entries,
                })
            }
        }
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
                value: format!("\"{value}\""),
                type_name: "String".to_string(),
            }),
            Expression::Parenthesized(inner) => self.evaluate(inner),
            Expression::Deref(_) => Err(anyhow!(
                "Pointer dereferencing not supported - values are automatically dereferenced"
            )),
            Expression::AddressOf { .. } => Err(anyhow!(
                "Address-of operator not supported in debugging context"
            )),
            Expression::MethodCall { base, method, args } => {
                self.evaluate_method_call(base, method, args)
            }
            Expression::FunctionCall { function, args } => {
                self.evaluate_function_call(function, args)
            }
        }
    }

    /// Evaluates an expression to a TypedPointer (for intermediate computation)
    pub fn evaluate_to_ref(&mut self, expr: &Expression) -> Result<TypedPointer> {
        match expr {
            Expression::Variable(name) => self.evaluate_variable_to_ref(name),
            Expression::FieldAccess { base, field } => {
                self.evaluate_field_access_to_ref(base, field)
            }
            Expression::Index { base, index } => self.evaluate_index_to_ref(base, index),
            // Method calls can potentially return pointers/references
            Expression::MethodCall { base, method, args } => {
                self.evaluate_method_call_to_ref(base, method, args)
            }
            Expression::FunctionCall { function, args } => {
                self.evaluate_function_call_to_ref(function, args)
            }
            // Literals, deref, and address-of don't have memory locations
            _ => Err(anyhow!(
                "Expression {:?} cannot be evaluated to a memory reference",
                expr
            )),
        }
    }

    /// Evaluate a method call and try to return a TypedPointer if it returns a complex pointer
    fn evaluate_method_call_to_ref(
        &mut self,
        base: &Expression,
        method: &str,
        args: &[Expression],
    ) -> Result<TypedPointer> {
        // First, get the base object's type and address
        let base_ref = self.evaluate_to_ref(base)?;
        let base_type = &base_ref.type_def;

        // Check if this is a synthetic method we can evaluate
        let synthetic_methods = get_synthetic_methods(base_type.layout.as_ref());
        let is_synthetic = synthetic_methods.iter().any(|m| m.name == method);

        if is_synthetic {
            // Synthetic methods don't typically return pointers we can use as arguments
            return Err(anyhow!(
                "Synthetic method '{}' cannot be used as an argument",
                method
            ));
        }

        // Try to execute the real method
        let discovered_methods = self.debug_info.discover_methods_for_pointer(&base_ref)?;

        // Find the specific method we want to call
        let method_info = discovered_methods
            .iter()
            .find(|m| m.name == method)
            .ok_or_else(|| {
                anyhow!(
                    "Method '{}' not found for type '{}' when trying to use as argument",
                    method,
                    base_ref.type_def.display_name()
                )
            })?;

        // Check if the method is callable (has an address)
        if method_info.address == 0 {
            return Err(anyhow!(
                "Method '{}' is not callable (no address found)",
                method
            ));
        }

        // Convert arguments to MethodArguments (recursive call)
        let mut method_args = Vec::new();
        for arg_expr in args {
            match self.convert_expression_to_method_arg(arg_expr) {
                Ok(method_arg) => method_args.push(method_arg),
                Err(e) => return Err(anyhow!("Failed to convert argument {:?}: {}", arg_expr, e)),
            }
        }

        // Calculate return type size from the type definition
        let return_type_size = method_info
            .return_type
            .as_ref()
            .and_then(|t| t.layout.size());

        // Send the ExecuteMethod event
        let event = EventRequest::ExecuteMethod {
            method_address: method_info.address,
            base_address: base_ref.address,
            args: method_args,
            return_type_size,
        };

        let response = self.conn.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::MethodResult { result } => match result {
                MethodCallResult::SimpleValue { .. } => Err(anyhow!(
                    "Method '{}' returns a simple value, not a pointer that can be used as an argument",
                    method
                )),
                MethodCallResult::ComplexPointer {
                    address,
                    size: _,
                    return_type: _,
                } => {
                    // Create a TypedPointer using the return type definition from the method
                    if let Some(return_type_def) = &method_info.return_type {
                        Ok(TypedPointer {
                            address,
                            type_def: return_type_def.clone(),
                        })
                    } else {
                        Err(anyhow!(
                            "Method '{}' returned a complex pointer but has no type definition",
                            method
                        ))
                    }
                }
            },
            EventResponseData::Error { message } => {
                Err(anyhow!("Method execution failed: {}", message))
            }
            _ => Err(anyhow!("Unexpected response type for ExecuteMethod")),
        }
    }

    /// Evaluate a function call and try to return a TypedPointer if it returns a complex pointer
    fn evaluate_function_call_to_ref(
        &mut self,
        function: &str,
        args: &[Expression],
    ) -> Result<TypedPointer> {
        // First, discover functions that match the given pattern
        let discovered_functions = self.debug_info.discover_functions(function)?;

        // Find the best matching function
        let function_info = discovered_functions.first().ok_or_else(|| {
            anyhow!(
                "Function '{}' not found when trying to use as argument",
                function
            )
        })?;

        // Check if the function is callable (has an address)
        if function_info.address == 0 {
            return Err(anyhow!(
                "Function '{}' is not callable (no address found)",
                function
            ));
        }

        // Check parameter count match
        if args.len() != function_info.parameters.len() {
            return Err(anyhow!(
                "Function '{}' expects {} arguments, but {} were provided when trying to use as argument",
                function,
                function_info.parameters.len(),
                args.len()
            ));
        }

        // Convert arguments to MethodArguments (recursive call)
        let mut function_args = Vec::new();
        for arg_expr in args {
            match self.convert_expression_to_method_arg(arg_expr) {
                Ok(function_arg) => function_args.push(function_arg),
                Err(e) => return Err(anyhow!("Failed to convert argument {:?}: {}", arg_expr, e)),
            }
        }

        // Calculate return type size from the type definition
        let return_type_size = function_info
            .return_type
            .as_ref()
            .and_then(|t| t.layout.size());

        // Send the ExecuteFunction event
        let event = EventRequest::ExecuteFunction {
            function_address: function_info.address,
            args: function_args,
            return_type_size,
        };

        let response = self.conn.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::FunctionResult { result } => match result {
                MethodCallResult::SimpleValue { .. } => Err(anyhow!(
                    "Function '{}' returns a simple value, not a pointer that can be used as an argument",
                    function
                )),
                MethodCallResult::ComplexPointer {
                    address,
                    size: _,
                    return_type: _,
                } => {
                    // Create a TypedPointer using the return type definition from the function
                    if let Some(return_type_def) = &function_info.return_type {
                        Ok(TypedPointer {
                            address,
                            type_def: return_type_def.clone(),
                        })
                    } else {
                        Err(anyhow!(
                            "Function '{}' returned a complex pointer but has no type definition",
                            function
                        ))
                    }
                }
            },
            EventResponseData::Error { message } => {
                Err(anyhow!("Function execution failed: {}", message))
            }
            _ => Err(anyhow!("Unexpected response type for ExecuteFunction")),
        }
    }

    fn evaluate_variable(&mut self, name: &str) -> Result<EvalResult> {
        // Try to get a TypedPointer, then convert to result
        let value_ref = self.evaluate_variable_to_ref(name)?;
        self.pointer_to_result(&value_ref)
    }

    fn evaluate_variable_to_ref(&mut self, name: &str) -> Result<TypedPointer> {
        let pc = self.get_pc()?;

        let var_info = self
            .debug_info
            .get_variable_at_pc(pc, name, &self.conn)?
            .with_context(|| format!("Failed to resolve variable '{name}'",))?;
        var_info
            .as_pointer()
            .ok_or_else(|| anyhow!("Variable '{name}' has no address"))
    }

    fn evaluate_field_access(&mut self, base: &Expression, field: &str) -> Result<EvalResult> {
        // Get the field as a TypedPointer, then convert to result
        let field_ref = self.evaluate_field_access_to_ref(base, field)?;
        self.pointer_to_result(&field_ref)
    }

    fn evaluate_field_access_to_ref(
        &mut self,
        base: &Expression,
        field: &str,
    ) -> Result<TypedPointer> {
        // First evaluate the base expression to a TypedPointer
        let base_ref = self.evaluate_to_ref(base)?;

        self.debug_info
            .get_field(base_ref.address, &base_ref.type_def, field)
    }

    fn evaluate_index_to_ref(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Result<TypedPointer> {
        let base_ref = self.evaluate_to_ref(base)?;

        // Check if the base type supports string indexing (HashMap, etc.)
        if self.supports_string_indexing(base_ref.type_def.layout.as_ref()) {
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

            Ok(element_info)
        } else {
            // Default to integer indexing
            let index_int = self.evaluate_to_int(index)?;
            self.debug_info
                .get_index_by_int(&base_ref, index_int, &self.conn)
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
    fn supports_string_indexing(&self, type_def: &Layout<impl Location>) -> bool {
        match type_def {
            Layout::Std(std_def) => {
                matches!(std_def, StdLayout::Map(_))
            }
            _ => false,
        }
    }

    /// Read an integer value from memory using a TypedPointer
    fn read_integer_from_memory(&self, pointer: &TypedPointer) -> Result<u64> {
        let value = self.debug_info.read_pointer(pointer, &self.conn)?;
        match value {
            Value::Scalar { value, .. } => {
                // Try to parse as different number formats
                if let Ok(num) = value.parse::<u64>() {
                    Ok(num)
                } else if let Some(hex_value) = value.strip_prefix("0x") {
                    u64::from_str_radix(hex_value, 16)
                        .with_context(|| format!("Failed to parse hex value: {value}"))
                } else {
                    Err(anyhow!("Could not parse integer value: {}", value))
                }
            }
            Value::Pointer(p) => self.read_integer_from_memory(&p),
            _ => Err(anyhow!("Expected scalar integer value, got: {:?}", value)),
        }
    }

    /// Read a string value from memory using a TypedPointer
    fn read_string_from_memory(&self, pointer: &TypedPointer) -> Result<String> {
        let value = self.debug_info.read_pointer(pointer, &self.conn)?;
        match value {
            Value::Scalar { value, .. } => {
                // For strings, the formatted value should be the string content
                // We might need to strip quotes depending on formatting
                let trimmed = value.trim_matches('"');
                Ok(trimmed.to_string())
            }
            Value::Pointer(_) => self.read_string_from_memory(pointer),
            _ => Err(anyhow!("Expected scalar string value, got: {:?}", value)),
        }
    }

    fn evaluate_index(&mut self, base: &Expression, index: &Expression) -> Result<EvalResult> {
        let pointer = self.evaluate_to_ref(base)?;

        // Check if the base type supports string indexing (HashMap, etc.)
        if self.supports_string_indexing(pointer.type_def.layout.as_ref()) {
            let key_string = self.evaluate_to_string(index)?;
            tracing::debug!(
                "Evaluating index with string key: {} for base type: {}",
                key_string,
                pointer.type_def.display_name()
            );
            // Use get_index_by_value with string key
            let key_value = Value::Scalar {
                ty: "String".to_string(),
                value: key_string,
            };
            let element_info = self.debug_info.get_index_by_value(
                pointer.address,
                &pointer.type_def,
                &key_value,
                &self.conn,
            )?;

            self.pointer_to_result(&element_info)
        } else {
            // Default to integer indexing
            let index_int = self.evaluate_to_int(index)?;
            let element_info = self
                .debug_info
                .get_index_by_int(&pointer, index_int, &self.conn)?;

            self.pointer_to_result(&element_info)
        }
    }

    /// Evaluate a method call expression
    fn evaluate_method_call(
        &mut self,
        base: &Expression,
        method: &str,
        args: &[Expression],
    ) -> Result<EvalResult> {
        // First, get the base object's type and address
        let base_ref = self.evaluate_to_ref(base)?;
        let base_type = &base_ref.type_def;

        // Check if this is a synthetic method we can evaluate
        let synthetic_methods = get_synthetic_methods(base_type.layout.as_ref());
        let is_synthetic = synthetic_methods.iter().any(|m| m.name == method);

        if is_synthetic {
            // Validate argument count for synthetic methods
            let method_info = synthetic_methods.iter().find(|m| m.name == method).unwrap();
            if method_info.takes_args && args.is_empty() {
                return Err(anyhow!("Method {}() expects arguments", method));
            } else if !method_info.takes_args && !args.is_empty() {
                return Err(anyhow!("Method {}() takes no arguments", method));
            }

            // Convert arguments to Values (currently we don't support args for synthetic methods)
            let arg_values = Vec::new();

            // Evaluate the synthetic method
            let result_value = evaluate_synthetic_method(
                base_ref.address,
                base_type,
                method,
                &arg_values,
                &self.conn,
            )?;

            // Convert Value to EvalResult
            Ok(EvalResult {
                value: format_value(&result_value),
                type_name: match &result_value {
                    Value::Scalar { ty, .. } => ty.clone(),
                    Value::Pointer(ptr) => ptr.type_def.display_name(),
                    _ => "unknown".to_string(),
                },
            })
        } else {
            // Try to execute the real method
            self.execute_real_method(base_ref, method, args)
        }
    }

    /// Execute a real method by calling it via LLDB
    fn execute_real_method(
        &mut self,
        pointer: TypedPointer,
        method: &str,
        args: &[Expression],
    ) -> Result<EvalResult> {
        // First, discover methods for this type to find the method address
        let discovered_methods = self.debug_info.discover_methods_for_pointer(&pointer)?;

        // Find the specific method we want to call
        let method_info = discovered_methods
            .iter()
            .find(|m| m.name == method)
            .ok_or_else(|| {
                anyhow!(
                    "Method '{}' not found for type '{}'",
                    method,
                    pointer.type_def.display_name()
                )
            })?;

        // Check if the method is callable (has an address)
        if method_info.address == 0 {
            return Err(anyhow!(
                "Method '{}' is not callable (no address found)",
                method
            ));
        }

        // Debug logging
        tracing::debug!(
            "Executing method '{}' at address {:#x} for type {}",
            method,
            method_info.address,
            pointer.type_def.display_name()
        );
        tracing::debug!("Method signature: {}", method_info.signature);
        tracing::debug!("Base object address: {:#x}", pointer.address);
        tracing::debug!("Number of arguments: {}", args.len());

        // Convert arguments to MethodArguments
        let mut method_args = Vec::new();
        for (i, arg_expr) in args.iter().enumerate() {
            match self.convert_expression_to_method_arg(arg_expr) {
                Ok(method_arg) => method_args.push(method_arg),
                Err(e) => {
                    // For methods, we don't have parameter type info in DiscoveredMethod
                    // Just provide the basic error
                    return Err(anyhow!(
                        "Failed to convert argument {} ({}): {}",
                        i + 1,
                        format!("{:?}", arg_expr),
                        e
                    ));
                }
            }
        }

        // Calculate return type size from the type definition
        let return_type_size = method_info
            .return_type
            .as_ref()
            .and_then(|t| t.layout.size());

        // Send the ExecuteMethod event
        let event = EventRequest::ExecuteMethod {
            method_address: method_info.address,
            base_address: pointer.address,
            args: method_args,
            return_type_size,
        };

        let response = self.conn.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::MethodResult { result } => match result {
                MethodCallResult::SimpleValue { value, return_type } => Ok(EvalResult {
                    value: value.to_string(),
                    type_name: return_type,
                }),
                MethodCallResult::ComplexPointer {
                    address,
                    size: _,
                    return_type: _,
                } => {
                    // Create a TypedPointer using the return type definition from the method
                    if let Some(return_type_def) = &method_info.return_type {
                        let typed_pointer = TypedPointer {
                            address,
                            type_def: return_type_def.clone(),
                        };

                        // Use the existing pointer_to_result method to get proper formatting
                        self.pointer_to_result(&typed_pointer)
                    } else {
                        // Fallback: no type definition available
                        Ok(EvalResult {
                            value: format!("<complex value at {address:#x}>"),
                            type_name: "unknown".to_string(),
                        })
                    }
                }
            },
            EventResponseData::Error { message } => {
                Err(anyhow!("Method execution failed: {}", message))
            }
            _ => Err(anyhow!("Unexpected response type for ExecuteMethod")),
        }
    }

    /// Evaluate a function call expression
    fn evaluate_function_call(
        &mut self,
        function: &str,
        args: &[Expression],
    ) -> Result<EvalResult> {
        // First, discover functions that match the given pattern
        let discovered_functions = self.debug_info.discover_functions(function)?;

        // Find the best matching function
        let function_info = discovered_functions.first().ok_or_else(|| {
            anyhow!(
                "Function '{}' not found. Available functions: {}",
                function,
                discovered_functions
                    .iter()
                    .take(5)
                    .map(|f| &f.name)
                    .join(", ")
            )
        })?;

        // Check if the function is callable (has an address)
        if function_info.address == 0 {
            return Err(anyhow!(
                "Function '{}' is not callable (no address found)",
                function
            ));
        }

        // Debug logging
        tracing::debug!(
            "Executing function '{}' at address {:#x}",
            function,
            function_info.address
        );
        tracing::debug!("Function signature: {}", function_info.signature);
        tracing::debug!("Number of arguments: {}", args.len());

        // Check parameter count match
        if args.len() != function_info.parameters.len() {
            return Err(anyhow!(
                "Function '{}' expects {} arguments, but {} were provided. Signature: {}",
                function,
                function_info.parameters.len(),
                args.len(),
                function_info.signature
            ));
        }

        // Convert arguments to MethodArguments
        let mut function_args = Vec::new();
        for (i, arg_expr) in args.iter().enumerate() {
            match self.convert_expression_to_method_arg(arg_expr) {
                Ok(function_arg) => function_args.push(function_arg),
                Err(e) => {
                    // Provide helpful context about parameter types
                    let param_info = if i < function_info.parameters.len() {
                        let param = &function_info.parameters[i];
                        let default_name = format!("arg{i}");
                        let param_name = param.name.as_deref().unwrap_or(&default_name);
                        format!(
                            " (parameter '{}' expects type: {})",
                            param_name,
                            param.type_def.display_name()
                        )
                    } else {
                        String::new()
                    };
                    return Err(anyhow!(
                        "Failed to convert argument {:?}{}: {}",
                        arg_expr,
                        param_info,
                        e
                    ));
                }
            }
        }

        // Calculate return type size from the type definition
        let return_type_size = function_info
            .return_type
            .as_ref()
            .and_then(|t| t.layout.size());

        // Send the ExecuteFunction event
        let event = EventRequest::ExecuteFunction {
            function_address: function_info.address,
            args: function_args,
            return_type_size,
        };

        let response = self.conn.conn.borrow_mut().send_event_request(event)?;

        match response {
            EventResponseData::FunctionResult { result } => match result {
                MethodCallResult::SimpleValue { value, return_type } => Ok(EvalResult {
                    value: value.to_string(),
                    type_name: return_type,
                }),
                MethodCallResult::ComplexPointer {
                    address,
                    size: _,
                    return_type: _,
                } => {
                    // Create a TypedPointer using the return type definition from the function
                    if let Some(return_type_def) = &function_info.return_type {
                        let typed_pointer = TypedPointer {
                            address,
                            type_def: return_type_def.clone(),
                        };

                        // Use the existing pointer_to_result method to get proper formatting
                        self.pointer_to_result(&typed_pointer)
                    } else {
                        // Fallback: no type definition available
                        Ok(EvalResult {
                            value: format!("<complex value at {address:#x}>"),
                            type_name: "unknown".to_string(),
                        })
                    }
                }
            },
            EventResponseData::Error { message } => {
                Err(anyhow!("Function execution failed: {}", message))
            }
            _ => Err(anyhow!("Unexpected response type for ExecuteFunction")),
        }
    }

    /// Convert an expression to a MethodArgument for execution
    fn convert_expression_to_method_arg(&mut self, expr: &Expression) -> Result<MethodArgument> {
        match expr {
            Expression::NumberLiteral(value) => Ok(MethodArgument {
                value: value.to_string(),
                arg_type: ArgumentType::Integer,
            }),
            Expression::StringLiteral(_value) => {
                // For string literals, we'd need to allocate them in the target process
                // For now, this is not supported
                Err(anyhow!(
                    "String literal arguments not yet supported - try passing a String variable instead"
                ))
            }
            // For all expressions that can be evaluated to a memory reference, use evaluate_to_ref
            Expression::Variable(_)
            | Expression::FieldAccess { .. }
            | Expression::Index { .. }
            | Expression::MethodCall { .. }
            | Expression::FunctionCall { .. } => {
                // Try to get a TypedPointer first (for variables, field access, indexing, and complex pointer returns)
                match self.evaluate_to_ref(expr) {
                    Ok(typed_pointer) => {
                        // We got a TypedPointer - pass the address as a pointer argument
                        Ok(MethodArgument {
                            value: format!("{:x}", typed_pointer.address), // No 0x prefix for LLDB
                            arg_type: ArgumentType::Pointer,
                        })
                    }
                    Err(_) => {
                        // evaluate_to_ref failed (likely because it returns a simple value)
                        // Fall back to evaluating and converting the result
                        let result = self.evaluate(expr)?;
                        self.eval_result_to_method_arg(result)
                    }
                }
            }
            _ => Err(anyhow!("Unsupported argument type: {:?}", expr)),
        }
    }

    /// Convert an EvalResult to a MethodArgument (for bare primitives)
    fn eval_result_to_method_arg(&self, result: EvalResult) -> Result<MethodArgument> {
        // Check if it's a pointer format "<Type @ 0xADDRESS>"
        if let Some(at_pos) = result.value.find(" @ 0x") {
            let addr_str = &result.value[at_pos + 4..result.value.len() - 1]; // Skip " @ 0x" and final ">"
            Ok(MethodArgument {
                value: addr_str.to_string(), // Use just the hex value without 0x prefix
                arg_type: ArgumentType::Pointer,
            })
        } else if result.value.starts_with("0x") {
            // Direct address value
            Ok(MethodArgument {
                value: result.value[2..].to_string(), // Remove 0x prefix
                arg_type: ArgumentType::Pointer,
            })
        } else if result.value == "true" || result.value == "false" {
            // Boolean value
            Ok(MethodArgument {
                value: if result.value == "true" { "1" } else { "0" }.to_string(),
                arg_type: ArgumentType::Bool,
            })
        } else if result.value.parse::<f64>().is_ok() {
            // Numeric value - could be integer or float
            if result.type_name.contains("f32") || result.type_name.contains("f64") {
                Ok(MethodArgument {
                    value: result.value,
                    arg_type: ArgumentType::Float,
                })
            } else {
                Ok(MethodArgument {
                    value: result.value,
                    arg_type: ArgumentType::Integer,
                })
            }
        } else {
            Err(anyhow!(
                "Cannot convert result to method argument: {} (type: {})",
                result.value,
                result.type_name
            ))
        }
    }
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

/// Information about a discovered function (for serialization to client)
#[derive(Debug, serde::Serialize, Clone)]
pub struct FunctionInfo {
    /// The function name (e.g., "main")
    pub name: String,
    /// The full function name with module path (e.g., "simple_test::main")
    pub full_name: String,
    /// The function signature (e.g., "fn main()")
    pub signature: String,
    /// The function address in memory
    pub address: u64,
    /// Whether this function can be called
    pub callable: bool,
    /// The module path components (e.g., ["simple_test"])
    pub module_path: Vec<String>,
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
        Value::Pointer(ptr) => {
            format!("<{} @ {:#x}>", ptr.type_def.display_name(), ptr.address)
        }
    }
}
