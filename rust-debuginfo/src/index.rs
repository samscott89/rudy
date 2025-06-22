//! Index building for fast debug info lookups

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use object::{Object, ObjectSymbol};

use crate::address_tree::{AddressTree, FunctionAddressInfo};
use crate::database::Db;
use crate::dwarf::{self, FunctionIndexEntry, Symbol, SymbolName};
use crate::file::{Binary, DebugFile, File, SourceFile, load};

#[salsa::tracked(returns(ref))]
pub fn discover_debug_files<'db>(
    db: &'db dyn Db,
    binary: Binary,
) -> BTreeMap<(String, Option<String>), DebugFile> {
    let binary_file = binary.file(db);
    let loaded_file = match load(db, binary_file) {
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
            (binary_file.path(db).to_string(), None),
            DebugFile::new(db, binary_file, false),
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

#[salsa::tracked(debug)]
pub struct Index<'db> {
    #[returns(ref)]
    pub data: IndexData<'db>,
}

impl<'db> Index<'db> {
    pub fn get_function(
        &self,
        db: &'db dyn Db,
        name: &SymbolName,
    ) -> Option<(DebugFile, FunctionIndexEntry)> {
        let file = *self
            .data(db)
            .symbol_to_file
            .get(&name.lookup_name)?
            .get(name)?;
        let indexed = dwarf::debug_file_index(db, file).data(db);
        indexed
            .functions
            .get(name)
            .cloned()
            .map(|entry| (file, entry))
    }
    // #[allow(dead_code)]
    // pub fn lookup_symbol(
    //     &self,
    //     db: &'db dyn Db,
    //     name: NameId<'db>,
    // ) -> Option<SymbolIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.symbols.get(&name).cloned()
    // }
    // #[allow(dead_code)]
    // pub fn lookup_module(
    //     &self,
    //     db: &'db dyn Db,
    //     name: NameId<'db>,
    // ) -> Option<ModuleIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.modules.get(&name).cloned()
    // }
    // #[allow(dead_code)]
    // pub fn lookup_type(&self, db: &'db dyn Db, name: NameId<'db>) -> Option<TypeIndexEntry<'db>> {
    //     let file = self.data(db).name_to_file.get(&name)?;
    //     let indexed = dwarf::debug_file_index(db, *file).data(db);
    //     indexed.types.get(&name).cloned()
    // }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexData<'db> {
    pub symbol_to_file: BTreeMap<String, BTreeMap<SymbolName, DebugFile>>,
    pub source_to_file: BTreeMap<SourceFile<'db>, Vec<DebugFile>>,
    pub base_address: BTreeMap<SymbolName, u64>,
    pub address_info: AddressTree,
}

impl<'db> IndexData<'db> {
    fn insert(&mut self, db: &dyn Db, symbol: SymbolName, file: DebugFile) {
        // insert the file into the index
        match self
            .symbol_to_file
            .entry(symbol.lookup_name.clone())
            .or_default()
            .entry(symbol)
        {
            Entry::Occupied(mut e) => {
                // if the file already exists, we need to check if the
                // base address is the same
                let existing_file = e.get_mut();
                let existing_file_path = existing_file.file(db).path(db);
                let file_path = file.file(db).path(db);
                if existing_file_path != file_path {
                    tracing::debug!(
                        "Symbol {} found in multiple files: {existing_file_path} and {file_path}",
                        e.key()
                    );
                }
            }
            Entry::Vacant(e) => {
                e.insert(file);
            }
        }
    }

    fn set_symbol_address(&mut self, name: SymbolName, address: u64) {
        // insert the file into the index
        match self.base_address.entry(name) {
            Entry::Occupied(e) => {
                // if the file already exists, we need to check if the
                // base address is the same
                let existing_file = e.get();
                if *existing_file != address {
                    tracing::warn!(
                        "Function {} has different base addresses: {existing_file:#x} and {address:#x}",
                        e.key()
                    );
                }
            }
            Entry::Vacant(e) => {
                e.insert(address);
            }
        }
    }
}

unsafe impl salsa::Update for IndexData<'_> {
    unsafe fn maybe_update(_: *mut Self, _: Self) -> bool {
        // IndexData should never change after creation
        todo!()
    }
}

#[salsa::tracked(returns(ref))]
fn get_symbol_map<'db>(
    db: &'db dyn Db,
    binary: Binary,
) -> BTreeMap<SymbolName, (u64, String, Option<String>)> {
    let binary_file = binary.file(db);
    let loaded_file = match load(db, binary_file) {
        Ok(file) => file,
        Err(e) => {
            db.report_critical(format!("Failed to load binary file: {e}"));
            return Default::default();
        }
    };

    let mut map: BTreeMap<SymbolName, (u64, String, Option<String>)> = BTreeMap::new();

    // find symbols in the main binary file
    let object = &loaded_file.object;
    for s in object.symbols() {
        let name = match s.name_bytes() {
            Ok(name) => name,
            Err(e) => {
                db.report_error(format!("Failed to parse symbol name: {e}"));
                continue;
            }
        };
        let symbol = Symbol::new(name.to_vec());
        match symbol.demangle() {
            Ok(demangled_name) => {
                map.insert(
                    demangled_name,
                    (s.address(), binary_file.path(db).to_string(), None),
                );
            }
            Err(e) => {
                tracing::debug!("Failed to demangle symbol {name:?}: {e}");
            }
        }
    }

    // find the symbol in the object file
    let object_map = object.object_map();
    for s in object_map.symbols() {
        let name_bytes = s.name();
        if name_bytes.is_empty() {
            // skip empty symbols
            continue;
        }
        let symbol = Symbol::new(name_bytes.to_vec());
        let demangled_name = match symbol.demangle() {
            Ok(name) => name,
            Err(e) => {
                tracing::debug!("Failed to demangle symbol {name_bytes:?}: {e}");
                continue;
            }
        };
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

        map.insert(
            demangled_name,
            (s.address(), file.to_string(), member.clone()),
        );
    }

    tracing::trace!("Symbols found in binary: {map:#?}",);

    map
}

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn debug_index<'db>(db: &'db dyn Db, binary: Binary) -> Index<'db> {
    // Builds a complete index of all debug information in the binary.
    // Note that today this works by scanning _all_ referenced debug files
    // and building an index for each file.
    // In the future, we may want to optimize this by only indexing
    // debug files _if_ they are known to contain relevant information
    // where relevant would be something like "references source files
    // that we care about"

    // get symbols from the binary file
    let symbol_map = get_symbol_map(db, binary);

    // discover all debug files associated with the binary
    let debug_files = discover_debug_files(db, binary);

    tracing::trace!("Debug files found: {debug_files:#?}",);

    // index each files and aggregate into a shared index
    let mut data = IndexData::default();
    let mut address_info = Vec::new();

    for ((path, member), debug_file) in debug_files.iter() {
        // let is_relocatable = debug_file.relocatable(db);
        let file = debug_file.file(db);
        let indexed = dwarf::debug_file_index(db, *debug_file).data(db);
        for name in indexed.functions.keys().chain(indexed.symbols.keys()) {
            data.insert(db, name.clone(), *debug_file);

            // if the debug file is relocatable, we need to
            // find the symbol address in the binary
            if let Some((address, symbol_path, symbol_member)) = symbol_map.get(name) {
                if symbol_path != path || symbol_member != member {
                    tracing::debug!(
                        "Symbol {name} found in file {} with address {address:#x} but also in binary with different path or member: {path} {member:?}",
                        file.path(db),
                    );
                } else {
                    data.set_symbol_address(name.clone(), *address);
                }
            }
        }

        // we handle functions separately because they have
        // address *range* information that we want to index
        for (name, entry) in &indexed.functions {
            let Some((start, end)) = entry.relative_address_range else {
                tracing::trace!(
                    "Function {name} in file {} does not have a valid address range",
                    file.path(db),
                );
                // function does not have a valid address range
                continue;
            };

            // insert the address range information for the function
            if let Some((base_address, symbol_path, symbol_member)) = symbol_map.get(name) {
                if symbol_path != path || symbol_member != member {
                    tracing::debug!(
                        "Function {name} found in file {} with address {base_address:#x} but also in binary with different path or member: {path} {member:?}",
                        file.path(db),
                    );
                } else {
                    address_info.push(FunctionAddressInfo {
                        start: *base_address,
                        end: base_address + end - start,
                        file: *debug_file,
                        name: name.clone(),
                    });
                }
            } else {
                // function not linked in binary -- this is fine
                tracing::trace!(
                    "Function {name} found in file {} but not linked in binary",
                    file.path(db),
                );
            }
        }

        for source in &indexed.sources {
            // insert the source file into the index
            data.source_to_file
                .entry(source.clone())
                .or_default()
                .push(*debug_file);
        }
    }

    // turn address info into an interval tree
    data.address_info = AddressTree::new(address_info);

    Index::new(db, data)
}

#[salsa::tracked]
pub fn find_closest_function<'db>(
    db: &'db dyn Db,
    binary: Binary,
    function_name: &'db str,
) -> Option<(SymbolName, DebugFile)> {
    // check if exact name exists in index
    let index = debug_index(db, binary);

    // otherwise, find the closest match by scanning the index
    // let name = function_name.name(db);
    // let module_prefix = function_name.path(db);

    let mut segments = function_name
        .split("::")
        .map(|s| s.to_owned())
        .collect::<Vec<String>>();
    let name = segments.pop()?;
    let module: Vec<String> = segments.iter().map(|s| s.to_string()).collect();

    let index_data = index.data(db);
    for (indexed_name, entry) in index_data.symbol_to_file.get(&name)? {
        // check if the name matches and the path ends with the module prefix
        if indexed_name.matches_name_and_module(&name, &module) {
            // get the function from the relevant index
            let indexed = dwarf::debug_file_index(db, *entry).data(db);
            if indexed.functions.contains_key(indexed_name) {
                return Some((indexed_name.clone(), *entry));
            } else {
                db.report_warning(format!(
                    "Function {indexed_name} found in file {} but not in index",
                    entry.file(db).path(db)
                ));
            }
        }
    }
    None
}

pub fn find_all_by_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: u64,
) -> Vec<&'db FunctionAddressInfo> {
    let index = debug_index(db, binary).data(db);

    index.address_info.query_address(address)
}

/// Resolve a type by name in the debug information
pub fn resolve_type<'db>(
    db: &'db dyn Db,
    binary: Binary,
    type_name: &str,
) -> anyhow::Result<Option<crate::typedef::TypeDef>> {
    let debug_files = discover_debug_files(db, binary);

    // Search through all debug files to find the type
    for debug_file in debug_files.values() {
        let indexed = dwarf::debug_file_index(db, *debug_file).data(db);

        // Look for types that match the given name
        let type_entry = indexed
            .types
            .iter()
            .find(|(name, _)| {
                // Try exact match first
                if name.name == type_name {
                    return true;
                }

                // For standard library types, also check if the simple name matches
                // e.g., "String" should match "alloc::string::String"
                if name.name == type_name
                    && (name.module.segments.contains(&"std".to_string())
                        || name.module.segments.contains(&"core".to_string())
                        || name.module.segments.contains(&"alloc".to_string()))
                {
                    return true;
                }

                // For generic types, check if the name starts with the type name
                // e.g., "Vec" should match "Vec<i32>"
                if name.name.starts_with(type_name)
                    && name.name.chars().nth(type_name.len()) == Some('<')
                {
                    return true;
                }

                false
            })
            .map(|(_, entry)| entry);

        if let Some(entries) = type_entry {
            for entry in entries {
                // Resolve the type using the DWARF resolution logic
                match crate::dwarf::resolve_type_offset(db, entry.die(db)) {
                    Ok(typedef) => {
                        // Successfully resolved the type
                        return Ok(Some(typedef));
                    }
                    Err(e) => {
                        tracing::debug!("Failed to resolve type '{}': {}", type_name, e);
                    }
                }
            }
        }
    }

    // Type not found in any debug file
    Ok(None)
}
