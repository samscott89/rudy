//! Output types that are part of the public interface of the debug info library.

use std::{collections::BTreeMap, fmt};

use rudy_dwarf::{function::SelfType, types::DieTypeDefinition};

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
pub struct Variable<'db> {
    pub name: String,
    pub value: Option<Value<'db>>,
    pub ty: DieTypeDefinition<'db>,
}

/// Variable metadata without resolved value - used for expression evaluation.
#[derive(Debug, Clone)]
pub struct VariableInfo<'db> {
    /// Variable name
    pub name: String,
    /// Memory address where variable is stored (if available)
    pub address: Option<u64>,
    /// Full type definition for the variable
    pub type_def: DieTypeDefinition<'db>,
}

impl<'db> VariableInfo<'db> {
    pub fn as_pointer(&self) -> Option<TypedPointer<'db>> {
        self.address.map(|address| TypedPointer {
            address,
            type_def: self.type_def.clone(),
        })
    }
}

/// A pointer to an entry in memory, with its type definition
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedPointer<'db> {
    /// Memory address where variable is stored (if available)
    pub address: u64,
    /// Full type definition for the variable
    pub type_def: DieTypeDefinition<'db>,
}

/// A value read from memory, supporting scalars, arrays, and structs.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Value<'db> {
    Scalar {
        ty: String,
        value: String,
    },
    Array {
        ty: String,
        items: Vec<Value<'db>>,
    },
    Struct {
        ty: String,
        fields: BTreeMap<String, Value<'db>>,
    },
    Tuple {
        ty: String,
        entries: Vec<Value<'db>>,
    },
    Map {
        ty: String,
        entries: Vec<(Value<'db>, Value<'db>)>,
    },
    Pointer(TypedPointer<'db>),
}

impl<'db> Value<'db> {
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
            Value::Pointer(ptr) => Value::Pointer(TypedPointer {
                address: ptr.address,
                type_def: ptr.type_def.clone(),
            }),
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

impl<'db> PartialOrd for Value<'db> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'db> Ord for Value<'db> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (
                Value::Scalar {
                    ty: ty1,
                    value: val1,
                },
                Value::Scalar {
                    ty: ty2,
                    value: val2,
                },
            ) => ty1.cmp(ty2).then_with(|| val1.cmp(val2)),
            (
                Value::Array {
                    ty: ty1,
                    items: items1,
                },
                Value::Array {
                    ty: ty2,
                    items: items2,
                },
            ) => ty1.cmp(ty2).then_with(|| items1.cmp(items2)),
            (
                Value::Struct {
                    ty: ty1,
                    fields: fields1,
                },
                Value::Struct {
                    ty: ty2,
                    fields: fields2,
                },
            ) => ty1.cmp(ty2).then_with(|| fields1.cmp(fields2)),
            (
                Value::Tuple {
                    ty: ty1,
                    entries: entries1,
                },
                Value::Tuple {
                    ty: ty2,
                    entries: entries2,
                },
            ) => ty1.cmp(ty2).then_with(|| entries1.cmp(entries2)),
            (
                Value::Map {
                    ty: ty1,
                    entries: entries1,
                },
                Value::Map {
                    ty: ty2,
                    entries: entries2,
                },
            ) => ty1.cmp(ty2).then_with(|| entries1.cmp(entries2)),
            (Value::Pointer(ptr1), Value::Pointer(ptr2)) => ptr1.address.cmp(&ptr2.address),
            // Define ordering between different variants
            (Value::Scalar { .. }, _) => Ordering::Less,
            (Value::Array { .. }, Value::Scalar { .. }) => Ordering::Greater,
            (Value::Array { .. }, _) => Ordering::Less,
            (Value::Struct { .. }, Value::Scalar { .. }) => Ordering::Greater,
            (Value::Struct { .. }, Value::Array { .. }) => Ordering::Greater,
            (Value::Struct { .. }, _) => Ordering::Less,
            (Value::Tuple { .. }, Value::Pointer(_)) => Ordering::Less,
            (Value::Tuple { .. }, Value::Map { .. }) => Ordering::Less,
            (Value::Tuple { .. }, _) => Ordering::Greater,
            (Value::Map { .. }, Value::Pointer(_)) => Ordering::Less,
            (Value::Map { .. }, _) => Ordering::Greater,
            (Value::Pointer(_), _) => Ordering::Greater,
        }
    }
}

/// Type information for a variable or field.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Type {
    pub name: String,
}

/// A resolved function with its address and parameter information.
#[derive(PartialEq, Eq, Clone)]
pub struct ResolvedFunction<'db> {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub params: Vec<Variable<'db>>,
}

impl fmt::Debug for ResolvedFunction<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedFunction")
            .field("name", &self.name)
            .field("address", &format!("{:#x}", self.address))
            .field("size", &format!("{:#x}", self.size))
            .field("params", &self.params)
            .finish()
    }
}

/// A discovered method with its metadata
#[derive(Debug, Clone)]
pub struct DiscoveredMethod<'db> {
    /// The method name (e.g., "len", "push")
    pub name: String,
    /// The full method name including type path
    pub full_name: String,
    /// The method signature as a string
    pub signature: String,
    /// The memory address of the method
    pub address: u64,
    /// Type of self parameter
    pub self_type: Option<SelfType>,
    /// Whether this method can be called (has an address)
    pub callable: bool,
    /// Whether this is a synthetic method (computed, not from debug info)
    pub is_synthetic: bool,
    /// The return type definition for creating TypedPointers
    pub return_type: Option<DieTypeDefinition<'db>>,
}
