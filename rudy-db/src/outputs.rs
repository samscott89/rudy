//! Output types that are part of the public interface of the debug info library.

use std::{collections::BTreeMap, fmt, sync::Arc};

use rudy_types::TypeLayout;

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

/// Variable metadata without resolved value - used for expression evaluation.
#[derive(Debug, Clone)]
pub struct VariableInfo {
    /// Variable name
    pub name: String,
    /// Memory address where variable is stored (if available)
    pub address: Option<u64>,
    /// Full type definition for the variable
    pub type_def: Arc<TypeLayout>,
}

/// A pointer to an entry in memory, with its type definition
#[derive(Debug, Clone)]
pub struct TypedPointer {
    /// Memory address where variable is stored (if available)
    pub address: u64,
    /// Full type definition for the variable
    pub type_def: Arc<TypeLayout>,
}

/// A value read from memory, supporting scalars, arrays, and structs.
#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord)]
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
    Tuple {
        ty: String,
        entries: Vec<Value>,
    },
    Map {
        ty: String,
        entries: Vec<(Value, Value)>,
    },
}

impl Value {
    pub(crate) fn map_type<F>(&self, type_map: F) -> Self
    where
        F: Fn(&str) -> String,
    {
        match self {
            Value::Scalar { value, ty } => Value::Scalar {
                ty: type_map(ty),
                value: value.clone(),
            },
            Value::Array { items, ty } => Value::Array {
                ty: type_map(ty),
                items: items.clone(),
            },
            Value::Tuple { entries, ty } => Value::Tuple {
                ty: type_map(ty),
                entries: entries.clone(),
            },
            Value::Struct { fields, ty } => Value::Struct {
                ty: type_map(ty),
                fields: fields.clone(),
            },
            Value::Map { entries, ty } => Value::Map {
                ty: type_map(ty),
                entries: entries.clone(),
            },
        }
    }

    /// Creates a new value with where the current type is prefixed with `prefix`.
    pub(crate) fn prefix_type<T: AsRef<str>>(&self, prefix: T) -> Self {
        let prefix = prefix.as_ref();
        self.map_type(|ty| format!("{prefix}{ty}"))
    }
    /// Creates a new value with where the current type is wrapped in `{new_ty}<{current_ty}>`.
    pub(crate) fn wrap_type<T: AsRef<str>>(&self, new_ty: T) -> Self {
        let new_ty = new_ty.as_ref();
        self.map_type(|ty| format!("{new_ty}<{ty}>"))
    }
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

/// Type of self parameter for methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum SelfType {
    /// Takes ownership: `self`
    Owned,
    /// Immutable reference: `&self`
    Borrowed,
    /// Mutable reference: `&mut self`
    BorrowedMut,
}

impl SelfType {
    /// Create SelfType from a resolved parameter type
    pub fn from_param_type(param_type: &TypeLayout) -> Self {
        use rudy_types::{PrimitiveLayout, ReferenceLayout};
        
        match param_type {
            TypeLayout::Primitive(PrimitiveLayout::Reference(ref_layout)) => {
                if ref_layout.mutable {
                    SelfType::BorrowedMut
                } else {
                    SelfType::Borrowed
                }
            },
            _ => SelfType::Owned,
        }
    }
}

/// A discovered method with its metadata
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredMethod {
    /// The method name (e.g., "len", "push")
    pub name: String,
    /// The full method name including type path
    pub full_name: String,
    /// The method signature as a string
    pub signature: String,
    /// The memory address of the method
    pub address: u64,
    /// Type of self parameter
    pub self_type: SelfType,
    /// Number of parameters including self
    pub parameter_count: usize,
    /// Whether this method can be called (has an address)
    pub callable: bool,
}
