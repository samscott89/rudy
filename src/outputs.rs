//! Output types that are part of the public interface of the debug info library.

use std::{collections::BTreeMap, fmt};

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

#[derive(Debug, PartialEq, Clone)]
pub struct ResolvedLocation {
    pub function: String,
    pub file: String,
    pub line: u64,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Variable {
    pub name: String,
    pub value: Option<Value>,
    pub ty: Option<Type>,
}

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

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Type {
    pub name: String,
}

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
