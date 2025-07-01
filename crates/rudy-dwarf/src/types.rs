//! Core types for DWARF debugging information

use rudy_types::{PrimitiveLayout, ReferenceLayout, TypeLayout};

use crate::file::SourceFile;

#[salsa::interned(debug)]
pub struct Position<'db> {
    pub file: SourceFile<'db>,
    pub line: u64,
    pub column: Option<u64>,
}

#[salsa::interned(debug)]
pub struct Address<'db> {
    pub address: u64,
}

/// Type of self parameter in a method
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SelfType {
    /// `self` - takes ownership
    Owned,
    /// `&self` - immutable reference
    Borrowed,
    /// `&mut self` - mutable reference
    BorrowedMut,
}

impl SelfType {
    pub fn from_param_type(param_type: &TypeLayout) -> Self {
        match param_type {
            TypeLayout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: true,
                ..
            })) => Self::BorrowedMut,
            TypeLayout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: false,
                ..
            })) => Self::Borrowed,
            _ => Self::Owned,
        }
    }
}
