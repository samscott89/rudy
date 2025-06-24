//! Shared parsing utilities for types and expressions
//!
//! This module provides common parsing functionality using the `unsynn` crate
//! for both type parsing (used in DWARF name resolution) and expression parsing
//! (used in LLDB integration).

pub mod types;
pub mod expressions;

pub use types::{parse_type, parse_symbol, Type, Path, ParsedSymbol};
pub use expressions::{parse_expression, Expression};