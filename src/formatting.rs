use std::collections::BTreeMap;
use std::fmt;

use itertools::Itertools;

use crate::dwarf::{self, CompilationUnitId};
use crate::types::{FunctionIndexEntry, NameId, SymbolIndexEntry, TypeIndexEntry};

fn normalize_path(path: String) -> String {
    // strip workspace dir
    let path = if let Some(pos) = path.find("/debug-test/") {
        let path = &path[pos + 12..];
        format!("$OUT_DIR/{}", path)
    } else {
        path
    };
    // strip rustlib folder prefix
    if let Some(pos) = path.find("/rustlib/") {
        let path = &path[pos + 9..];
        format!("$RUSTLIB/{}", path)
    } else {
        path
    }
}

struct FunctionNameToDieDebug<'db>(&'db BTreeMap<NameId<'db>, FunctionIndexEntry<'db>>);

impl<'db> fmt::Debug for FunctionNameToDieDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (name, entry) in self
                .0
                .iter()
                .map(|(name, entry)| {
                    let name = name.as_path(db);
                    let entry = normalize_path(entry.die(db).as_path_ref(db));
                    (name, entry)
                })
                .sorted()
            {
                d.entry(&name, &entry);
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}

struct SymbolNameToDieDebug<'db>(&'db BTreeMap<NameId<'db>, SymbolIndexEntry<'db>>);

impl<'db> fmt::Debug for SymbolNameToDieDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (name, entry) in self
                .0
                .iter()
                .map(|(name, entry)| {
                    let name = name.as_path(db);
                    let entry = normalize_path(entry.die(db).as_path_ref(db));
                    (name, entry)
                })
                .sorted()
            {
                d.entry(&name, &entry);
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}

struct TypeNameToDieDebug<'db>(&'db BTreeMap<NameId<'db>, TypeIndexEntry<'db>>);

impl<'db> fmt::Debug for TypeNameToDieDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (name, entry) in self
                .0
                .iter()
                .map(|(name, entry)| {
                    let name = name.as_path(db);
                    let entry = normalize_path(entry.die(db).as_path_ref(db));
                    (name, entry)
                })
                .sorted()
            {
                d.entry(&name, &entry);
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}

// struct DieToTypeDebug<'db>(&'db BTreeMap<DieEntryId<'db>, NameId<'db>>);

// impl<'db> fmt::Debug for DieToTypeDebug<'db> {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         salsa::with_attached_database(|db| {
//             let db = db.as_view();
//             let mut d = f.debug_map();
//             for (entry, name) in self.0 {
//                 d.entry(&entry.as_path_ref(db), &name.as_path(db));
//             }
//             d.finish()?;

//             Ok(())
//         })
//         .unwrap_or(Ok(()))
//     }
// }

struct CuToBaseAddrDebug<'db>(&'db BTreeMap<CompilationUnitId<'db>, u64>);
impl<'db> fmt::Debug for CuToBaseAddrDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (cu, addr) in self
                .0
                .iter()
                .map(|(cu, addr)| {
                    let cu = normalize_path(cu.as_path_ref(db));
                    let addr = format!("{addr:#x}");
                    (cu, addr)
                })
                .sorted()
            {
                d.entry(&cu, &addr);
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}

struct AddressRangeToCuDebug<'db>(&'db [(u64, u64, CompilationUnitId<'db>)]);
impl<'db> fmt::Debug for AddressRangeToCuDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (start, end, cu) in self.0 {
                d.entry(
                    &format!("{start:#x}..{end:#x}"),
                    &normalize_path(cu.as_path_ref(db)),
                );
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}
struct AddressRangeToFunctionDebug<'db>(&'db [(u64, u64, NameId<'db>)]);
impl<'db> fmt::Debug for AddressRangeToFunctionDebug<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        salsa::with_attached_database(|db| {
            let db = db.as_view();
            let mut d = f.debug_map();
            for (start, end, name) in self.0 {
                d.entry(&format!("{start:#x}..{end:#x}"), &name.as_path(db));
            }
            d.finish()?;

            Ok(())
        })
        .unwrap_or(Ok(()))
    }
}

impl<'db> fmt::Debug for dwarf::IndexData<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            function_name_to_die,
            symbol_name_to_die,
            type_name_to_die,
            die_to_type_name: _,
            cu_to_base_addr,
            address_range_to_cu,
            address_range_to_function,
            file_to_cu: _,
        } = self;

        f.debug_struct("IndexData")
            .field(
                "function_name_to_die",
                &FunctionNameToDieDebug(&function_name_to_die),
            )
            .field(
                "symbol_name_to_die",
                &SymbolNameToDieDebug(&symbol_name_to_die),
            )
            .field("type_name_to_die", &TypeNameToDieDebug(&type_name_to_die))
            // .field("die_to_type_name", &DieToTypeDebug(&die_to_type_name))
            .field("cu_to_base_addr", &CuToBaseAddrDebug(&cu_to_base_addr))
            .field(
                "address_range_to_cu",
                &AddressRangeToCuDebug(&address_range_to_cu),
            )
            .field(
                "address_range_to_function",
                &AddressRangeToFunctionDebug(&address_range_to_function),
            )
            .finish()
    }
}
