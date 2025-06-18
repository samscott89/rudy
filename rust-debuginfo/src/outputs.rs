//! Output types that are part of the public interface of the debug info library.

use std::{collections::BTreeMap, fmt};

/// A resolved memory address from a source location.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAddress {
    pub address: u64,
}

impl fmt::Debug for ResolvedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedAddress")
            .field("address", &format!("{:#x}", self.address))
            .finish()
    }
}

/// Source location information resolved from a memory address.
#[derive(Debug, PartialEq, Clone)]
pub struct ResolvedLocation {
    pub function: String,
    pub file: String,
    pub line: u64,
}

/// A variable with its type and optionally its runtime value.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Variable {
    pub name: String,
    pub value: Option<Value>,
    pub ty: Option<Type>,
}

/// A value read from memory, supporting scalars, arrays, and structs.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Value {
    Scalar {
        ty: String,
        value: String,
    },
    Array {
        ty: String,
        items: Vec<Value>,
    },
    Struct {
        ty: String,
        fields: BTreeMap<String, Value>,
    },
}

/// Type information for a variable or field.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Type {
    pub name: String,
}

/// A resolved function with its address and parameter information.
#[derive(PartialEq, Eq, Clone)]
pub struct ResolvedFunction {
    pub name: String,
    pub address: u64,
    pub params: Vec<Variable>,
}

impl fmt::Debug for ResolvedFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedFunction")
            .field("name", &self.name)
            .field("address", &format!("{:#x}", self.address))
            .field("params", &self.params)
            .finish()
    }
}
