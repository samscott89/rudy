//! Expression parsing and evaluation
//!
//! This module handles parsing Rust expressions and evaluating them
//! using debug information.

use anyhow::{anyhow, Result};

/// Represents a parsed expression
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// Simple variable reference (e.g., `foo`)
    Variable(String),
    
    // Future expression types:
    // /// Field access (e.g., `foo.bar`)
    // FieldAccess {
    //     base: Box<Expression>,
    //     field: String,
    // },
    // /// Array/slice indexing (e.g., `arr[5]`)
    // Index {
    //     base: Box<Expression>,
    //     index: Box<Expression>,
    // },
    // /// Dereference (e.g., `*ptr`)
    // Deref(Box<Expression>),
    // /// Method call (e.g., `foo.method()`)
    // MethodCall {
    //     base: Box<Expression>,
    //     method: String,
    //     args: Vec<Expression>,
    // },
}

/// Parse a string into an Expression
pub fn parse(input: &str) -> Result<Expression> {
    let input = input.trim();
    
    // For now, only support simple variable names
    if is_valid_identifier(input) {
        Ok(Expression::Variable(input.to_string()))
    } else {
        Err(anyhow!("Invalid expression: '{}'. Only simple variable names are supported for now.", input))
    }
}

/// Check if a string is a valid Rust identifier
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    
    // First character must be letter or underscore
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    
    // Remaining characters must be letters, digits, or underscores
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_variable() {
        assert_eq!(parse("foo").unwrap(), Expression::Variable("foo".to_string()));
        assert_eq!(parse("_bar").unwrap(), Expression::Variable("_bar".to_string()));
        assert_eq!(parse("var123").unwrap(), Expression::Variable("var123".to_string()));
        assert_eq!(parse("  user  ").unwrap(), Expression::Variable("user".to_string()));
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse("").is_err());
        assert!(parse("123foo").is_err());
        assert!(parse("foo.bar").is_err()); // Not supported yet
        assert!(parse("foo[0]").is_err());  // Not supported yet
        assert!(parse("*ptr").is_err());    // Not supported yet
    }
}