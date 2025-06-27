//! Shared parsing utilities for types and expressions
//!
//! This module provides common parsing functionality using the `unsynn` crate
//! for both type parsing (used in DWARF name resolution) and expression parsing
//! (used in LLDB integration).

pub mod expressions;
pub mod types;

pub use expressions::{Expression, parse_expression};
pub use types::{ParsedSymbol, Path, Type, parse_symbol, parse_type};
