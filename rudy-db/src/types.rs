//! Some internal DB types

use rudy_types::{PrimitiveLayout, ReferenceLayout, TypeLayout};

use rudy_dwarf::SourceFile;

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
