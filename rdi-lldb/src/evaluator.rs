//! Expression evaluation using debug information
//!
//! This module evaluates parsed expressions by looking up debug information
//! and reading memory through event callbacks.

use anyhow::Result;
use rust_debuginfo::DebugInfo;

use crate::expression::Expression;
use crate::protocol::{EventRequest, EventResponseData};

/// Context needed for expression evaluation
pub struct EvalContext<'db> {
    /// Debug information for the binary
    pub debug_info: &'db DebugInfo<'db>,
    /// Current program counter
    pub pc: u64,
    /// Current stack pointer
    pub sp: u64,
    /// Current frame pointer
    pub fp: u64,
}

/// Result of evaluating an expression
#[derive(Debug)]
pub struct EvalResult {
    /// The evaluated value (formatted for display)
    pub value: String,
    /// The type of the value
    pub type_name: String,
    /// Pretty-printed representation
    pub pretty: String,
}

/// Evaluates an expression, potentially generating events for the client
pub fn evaluate(
    expr: &Expression,
    context: &EvalContext,
    request_id: u64,
) -> Result<EvaluationState> {
    match expr {
        Expression::Variable(name) => evaluate_variable(name, context, request_id),
    }
}

/// State machine for expression evaluation
pub enum EvaluationState {
    /// Evaluation needs data from the client
    NeedEvent {
        /// The event to send to the client
        event: EventRequest,
        /// Continuation function to call with the response
        continuation: Box<dyn FnOnce(EventResponseData) -> Result<EvaluationState>>,
    },
    /// Evaluation is complete
    Complete(EvalResult),
}

fn evaluate_variable(
    name: &str,
    context: &EvalContext,
    _request_id: u64,
) -> Result<EvaluationState> {
    // TODO: Actually look up the variable in debug info
    // For now, just return a placeholder

    // In a real implementation, this would:
    // 1. Use context.debug_info to find variables at context.pc
    // 2. Look for a variable with the given name
    // 3. Get its type and location (register or memory)
    // 4. Return NeedEvent to read the value
    // 5. In the continuation, format the value based on its type

    Ok(EvaluationState::Complete(EvalResult {
        value: format!("<variable '{}' at {:#x}>", name, context.pc),
        type_name: "Unknown".to_string(),
        pretty: format!("{} = <not yet implemented>", name),
    }))
}

// Future implementations:
// - evaluate_field_access
// - evaluate_index
// - evaluate_deref
// - evaluate_method_call
