//! Index building for fast debug info lookups

use std::collections::{BTreeMap, HashMap};

use object::{Object, ObjectSymbol};

use crate::database::Db;
use crate::dwarf;
use crate::file::{FileId, SourceFile};
use crate::types::{
    FunctionIndexEntry, NameId, Symbol, SymbolIndexEntry, TypeIndexEntry, demangle,
};

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(return_ref)]
pub fn index<'db>(db: &'db dyn Db) -> dwarf::Index<'db> {
    let binary_file = db.binary_file();
    let Some(object) = binary_file.object() else {
        return dwarf::Index::new(db, Default::default());
    };

    // initialize structs
    let mut function_name_to_die: BTreeMap<NameId<'_>, FunctionIndexEntry<'_>> = Default::default();
    let mut symbol_name_to_die: BTreeMap<NameId<'_>, SymbolIndexEntry<'_>> = Default::default();
    let mut type_name_to_die: BTreeMap<NameId<'_>, TypeIndexEntry<'_>> = Default::default();
    let mut die_to_type_name: BTreeMap<dwarf::Die<'_>, NameId<'_>> = Default::default();
    let mut cu_to_base_addr: BTreeMap<dwarf::CompilationUnitId<'_>, u64> = Default::default();
    let mut address_range_to_cu: Vec<(u64, u64, dwarf::CompilationUnitId<'_>)> = Default::default();
    let mut address_range_to_function: Vec<(u64, u64, NameId<'_>)> = Default::default();
    let mut file_to_cu: BTreeMap<SourceFile<'_>, Vec<dwarf::CompilationUnitId<'_>>> =
        Default::default();

    let mut names_by_file: HashMap<FileId<'db>, BTreeMap<Vec<u8>, _>> = HashMap::new();

    let object_map = object.object_map();
    for s in object_map.symbols() {
        let symbol = Symbol::new(db, s.name());
        let demangled_name = demangle(db, symbol);
        let file = s.object(&object_map).path();
        let file = match std::str::from_utf8(file) {
            Ok(p) => p,
            Err(e) => {
                db.report_critical(format!("Failed to parse object file path: {file:?}: {e}"));
                continue;
            }
        };
        let member = s
            .object(&object_map)
            .member()
            .and_then(|m| match std::str::from_utf8(m) {
                Ok(m) => Some(m.to_string()),
                Err(e) => {
                    db.report_critical(format!("Failed to parse object file member: {m:?}: {e}"));
                    None
                }
            });
        let file = FileId::new(db, file.to_string(), member, true);
        names_by_file.entry(file).or_default().insert(
            // trim the leading `_` character that macos adds when using STAB entries
            s.name()[1..].to_vec(),
            (s.address(), demangled_name),
        );
    }

    // append names from the root binary (if it has any)
    for symbol in object.symbols() {
        let name = symbol.name_bytes().unwrap();
        if name.is_empty() {
            tracing::debug!("empty symbol: {symbol:#?}");
            continue;
        }
        let symbol_name = Symbol::new(db, name);
        let demangled_name = demangle(db, symbol_name);

        let file = binary_file.file_id(db);
        names_by_file
            .entry(file)
            .or_default()
            .insert(name.to_vec(), (symbol.address(), demangled_name));
    }

    for (file_id, names) in names_by_file {
        let file_entries = dwarf::index_symbols(db, file_id, names);

        function_name_to_die.extend(file_entries.function_name_to_die);
        symbol_name_to_die.extend(file_entries.symbol_name_to_die);
        address_range_to_function.extend(file_entries.address_range_to_function);
        cu_to_base_addr.extend(file_entries.cu_to_base_addr);

        let (name_to_die, die_to_name) = dwarf::index_types(db, file_id);
        type_name_to_die.extend(name_to_die);
        die_to_type_name.extend(die_to_name);

        let roots = dwarf::parse_roots(db, file_id);
        for root in roots {
            let cu = root.cu(db);
            if let Some(base_addr) = cu_to_base_addr.get(&root.cu(db)) {
                let (start, end) = root.address_range(db);
                address_range_to_cu.push((base_addr + start, base_addr + end, cu));
            }

            for file in root.files(db) {
                file_to_cu.entry(*file).or_default().push(cu)
            }
        }
    }

    // sort the lists
    address_range_to_function.sort_unstable();
    address_range_to_cu.sort_unstable();

    dwarf::Index::new(
        db,
        dwarf::IndexData {
            function_name_to_die,
            symbol_name_to_die,
            type_name_to_die,
            die_to_type_name,
            cu_to_base_addr,
            address_range_to_cu,
            address_range_to_function,
            file_to_cu,
        },
    )
}
