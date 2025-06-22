//! Index building for fast debug info lookups

use std::collections::BTreeMap;

use anyhow::Context;

use crate::address_tree::FunctionAddressInfo;
use crate::database::Db;
use crate::dwarf::{self, FunctionIndexEntry, SymbolName};
use crate::file::{Binary, DebugFile, SourceFile};
use crate::index::symbols::{DebugFiles, SymbolIndex};

mod symbols;

#[salsa::tracked(debug)]
pub struct Index<'db> {
    pub debug_files: DebugFiles,
    pub symbol_index: SymbolIndex,
    pub indexed_debug_files: Vec<DebugFile>,
    pub source_to_file: BTreeMap<SourceFile<'db>, Vec<DebugFile>>,
}

impl<'db> Index<'db> {
    pub fn get_function(
        &self,
        db: &'db dyn Db,
        name: &SymbolName,
    ) -> Option<(DebugFile, FunctionIndexEntry)> {
        let sym = self
            .symbol_index(db)
            .get_functions_by_lookup_name(&name.lookup_name)?
            .get(name)?
            .clone();
        let indexed = dwarf::debug_file_index(db, sym.debug_file).data(db);
        indexed
            .functions
            .get(name)
            .cloned()
            .map(|entry| (sym.debug_file, entry))
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

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn debug_index<'db>(db: &'db dyn Db, binary: Binary) -> Index<'db> {
    let Ok((debug_files, symbol_index)) = symbols::index_symbol_map(db, binary)
        .with_context(|| {
            format!(
                "Failed to index debug files for binary: {}",
                binary.file(db).path(db)
            )
        })
        .inspect_err(|e| {
            db.report_critical(format!("Failed to index debug files: {e}"));
        })
    else {
        return Index::new(
            db,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
    };

    tracing::trace!("Debug files found: {debug_files:#?}",);

    // TODO: get "main debug files" (i.e. the binary itself, or the debug file adjacent to it)
    // and insert into the vec + index the symbols

    Index::new(db, debug_files, symbol_index, vec![], Default::default())
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

    for (indexed_name, entry) in index.symbol_index(db).get_functions_by_lookup_name(&name)? {
        // check if the name matches and the path ends with the module prefix
        if indexed_name.matches_name_and_module(&name, &module) {
            return Some((indexed_name.clone(), entry.debug_file));
        }
    }
    None
}

pub fn find_all_by_address<'db>(
    db: &'db dyn Db,
    binary: Binary,
    address: u64,
) -> Vec<&'db FunctionAddressInfo> {
    // first, find the closest function by address
    let index = debug_index(db, binary).symbol_index(db);
    let Some(function) = index.function_at_address(address) else {
        return vec![];
    };

    // then, query the debug file index to find the actual matching addresses
    let debug_file = function.debug_file;
    let indexed = dwarf::debug_file_index(db, debug_file).data(db);
    indexed.function_addresses.query_address(address)
}

/// Resolve a type by name in the debug information
pub fn resolve_type<'db>(
    db: &'db dyn Db,
    binary: Binary,
    type_name: &str,
) -> anyhow::Result<Option<crate::typedef::TypeDef>> {
    let index = debug_index(db, binary);

    // Search through all debug files to find the type
    for debug_file in index.indexed_debug_files(db) {
        let indexed = dwarf::debug_file_index(db, debug_file).data(db);

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
