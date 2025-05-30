//! Index building for fast debug info lookups

use std::collections::{BTreeMap, HashMap};

use object::{Object, ObjectSymbol};

use crate::database::Db;
use crate::dwarf;
use crate::file::{Binary, DebugFile, File, SourceFile, load};
use crate::types::{
    FunctionIndexEntry, NameId, Symbol, SymbolIndexEntry, TypeIndexEntry, demangle,
};

#[salsa::tracked(returns(ref))]
fn discover_debug_files<'db>(
    db: &'db dyn Db,
    binary: File,
) -> BTreeMap<(String, Option<String>), DebugFile<'db>> {
    let loaded_file = match load(db, binary) {
        Ok(file) => file,
        Err(e) => {
            db.report_critical(format!("Failed to load binary file: {e}"));
            return Default::default();
        }
    };
    let object = &loaded_file.object;

    let mut debug_files = BTreeMap::new();

    if object.has_debug_symbols() {
        // if the main binary has debug symbols, we'll use it direclty
        debug_files.insert(
            (binary.path(db).to_string(), None),
            DebugFile::new(db, binary, false),
        );
    }

    // find any debug files associated with the binary
    let object_map = object.object_map();
    for obj in object_map.objects() {
        let file = obj.path();
        let file = match String::from_utf8(file.to_vec()) {
            Ok(p) => p,
            Err(e) => {
                let lossy_file = String::from_utf8_lossy(file);
                db.report_warning(format!("ignoring non-UTF8 file: {lossy_file}\n{e}"));
                continue;
            }
        };
        let member = obj
            .member()
            .and_then(|m| match String::from_utf8(m.to_vec()) {
                Ok(m) => Some(m),
                Err(e) => {
                    let lossy_file = String::from_utf8_lossy(m);
                    db.report_warning(format!(
                        "ignoring non-UTF8 archive member: {lossy_file}\n{e}"
                    ));
                    None
                }
            });

        match debug_files.entry((file.clone(), member.clone())) {
            std::collections::btree_map::Entry::Occupied(_) => continue,
            std::collections::btree_map::Entry::Vacant(e) => {
                let file = match File::build(db, file.clone(), member.clone()) {
                    Ok(file) => file,
                    Err(e) => {
                        db.report_critical(format!(
                            "Failed to load debug file {file} {member:?}: {e}",
                        ));
                        continue;
                    }
                };
                e.insert(DebugFile::new(db, file, false));
            }
        };
    }

    debug_files
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn build_index<'db>(db: &'db dyn Db, binary_file: File) -> dwarf::Index<'db> {
    // let binary_file = binary.file(db);

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

    let mut names_by_file: HashMap<File, BTreeMap<Vec<u8>, _>> = HashMap::new();

    // first, discover all debug files associated with the binary
    let debug_files = discover_debug_files(db, binary_file);

    // first load the binary file itself to get a list of all symbols and
    // associated file paths
    // we need to do this in a block to avoid holding onto the `Ref` (which is a read lock
    // on the file map)
    {
        let Ok(file) = load(db, binary_file) else {
            db.report_critical(format!("Failed to load file: {}", binary_file.path(db)));
            return dwarf::Index::new(db, Default::default());
        };
        let object = &file.object;

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
            let member =
                s.object(&object_map)
                    .member()
                    .and_then(|m| match std::str::from_utf8(m) {
                        Ok(m) => Some(m.to_string()),
                        Err(e) => {
                            db.report_critical(format!(
                                "Failed to parse object file member: {m:?}: {e}"
                            ));
                            None
                        }
                    });
            let Some(file) = debug_files.get(&(file.to_string(), member.clone())) else {
                tracing::debug!("No debug file found {file:?} with member {member:?}");
                continue;
            };
            names_by_file.entry(file.file(db)).or_default().insert(
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

            names_by_file
                .entry(binary_file)
                .or_default()
                .insert(name.to_vec(), (symbol.address(), demangled_name));
        }
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
