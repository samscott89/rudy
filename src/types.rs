//! Core types used throughout the debug info system

use itertools::Itertools;

use crate::database::Db;
use crate::dwarf;

#[salsa::interned]
pub struct NameId<'db> {
    #[return_ref]
    pub path: Vec<String>,
    #[return_ref]
    pub name: String,
}

impl<'db> NameId<'db> {
    pub fn as_path(&self, db: &'db dyn Db) -> String {
        let name = self.name(db);
        let path = self.path(db).iter().join("::");
        if path.is_empty() {
            name.to_string()
        } else {
            format!("{path}::{name}")
        }
    }
}

#[salsa::interned]
pub struct Symbol<'db> {
    #[return_ref]
    pub name_bytes: Vec<u8>,
}

#[salsa::tracked]
pub fn demangle<'db>(db: &'db dyn Db, sym: Symbol<'db>) -> NameId<'db> {
    let name_str = match std::str::from_utf8(sym.name_bytes(db).as_ref()) {
        Ok(name_str) => name_str,
        Err(_) => {
            tracing::warn!("Failed to demangle symbol: {:?}", sym.name_bytes(db));
            db.report_critical(format!(
                "Failed to parse symbol bytes to string: {:?}",
                sym.name_bytes(db)
            ));
            return NameId::new(db, vec![], "<invalid>".to_string());
        }
    };
    let demangled = rustc_demangle::demangle(name_str.as_ref());
    // return the demangled name as a string, without the trailing hash
    let demangled = format!("{demangled:#}");
    let mut split: Vec<String> = demangled.split("::").map(|s| s.to_owned()).collect();
    let name = split.pop().unwrap_or_else(|| {
        db.report_error(format!("Invalid empty symbol name: {demangled}",));
        "<invalid>".to_string()
    });
    NameId::new(db, split, name)
}

#[salsa::tracked]
pub struct FunctionIndexEntry<'db> {
    pub die: dwarf::Die<'db>,
}

#[salsa::tracked]
pub struct SymbolIndexEntry<'db> {
    pub address: u64,
    pub die: dwarf::Die<'db>,
}

#[salsa::tracked]
pub struct TypeIndexEntry<'db> {
    pub die: dwarf::Die<'db>,
}

#[salsa::interned]
pub struct Position<'db> {
    pub file: String,
    pub line: u64,
    pub column: Option<u64>,
}

#[salsa::interned]
pub struct Address<'db> {
    pub address: u64,
}
