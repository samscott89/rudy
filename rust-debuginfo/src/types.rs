//! Internal types for the database.

use itertools::Itertools;

use crate::{database::Db, file::SourceFile};

#[salsa::interned(debug)]
#[derive(Ord, PartialOrd)]
pub struct NameId<'db> {
    #[returns(ref)]
    pub path: Vec<String>,
    #[returns(ref)]
    pub name: String,
    #[returns(ref)]
    pub generics: Vec<NameId<'db>>,
}

impl<'db> NameId<'db> {
    pub fn parse_string_in_module(path: Vec<String>, s: &str, db: &'db dyn Db) -> Self {
        let (name, generics) = if let Some((name, generics)) = s.split_once('<') {
            if generics.is_empty() {
                tracing::error!("Invalid generic type: {generics} in {s}");
            }
            // NOTE: this won't support doubly-nested generics
            let generics = generics[..generics.len() - 1]
                .split(',')
                .map(|s| NameId::parse_string(s, db))
                .collect();
            (name, generics)
        } else {
            (s, vec![])
        };

        NameId::new(db, path, name.to_string(), generics)
    }
    pub fn parse_string(s: &str, db: &'db dyn Db) -> Self {
        tracing::info!("Parsing symbol: {s}");
        let (name, generics) = if let Some((name, generics)) = s.split_once('<') {
            if generics.is_empty() {
                tracing::error!("Invalid generic type: {generics} in {s}");
            }
            // NOTE: this won't support doubly-nested generics
            let generics = generics[..generics.len() - 1]
                .split(',')
                .map(|s| NameId::parse_string(s, db))
                .collect();
            (name, generics)
        } else {
            (s, vec![])
        };

        let mut path: Vec<String> = name.split("::").map(|s| s.to_owned()).collect();
        let name = path.pop().unwrap_or_else(|| {
            db.report_error(format!("Invalid empty symbol name: {s}",));
            "<invalid>".to_string()
        });
        NameId::new(db, path, name, generics)
    }

    pub fn as_path<Db>(&self, db: &'db Db) -> String
    where
        Db: salsa::Database + ?Sized,
    {
        let name = self.name(db);
        let path = self.path(db);
        let path = if path.is_empty() {
            String::new()
        } else {
            path.join("::") + "::"
        };
        let generics = self.generics(db);
        let generics = if generics.is_empty() {
            String::new()
        } else {
            format!(
                "<{}>",
                generics
                    .iter()
                    .map(|g| g.as_path(db))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!("{path}{name}{generics}")
    }
}

#[salsa::interned]
pub struct Symbol<'db> {
    #[returns(ref)]
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
            return NameId::new(db, vec![], "<invalid>".to_string(), vec![]);
        }
    };
    let demangled = rustc_demangle::demangle(name_str.as_ref());
    // return the demangled name as a string, without the trailing hash
    NameId::parse_string(&demangled.to_string(), db)
}

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
