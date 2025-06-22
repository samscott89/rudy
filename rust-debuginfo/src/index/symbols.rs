//! Symbol-based indexing for fast debug info lookups

use anyhow::{Context, Result};
use object::{Object, ObjectSymbol};

use crate::database::Db;
use crate::dwarf::{RawSymbol, SymbolName};
use crate::file::{Binary, DebugFile, File, load};
use std::collections::BTreeMap;

use itertools::Itertools;

/// Information about a symbol from the symbol table
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub name: SymbolName,
    pub address: u64,
    pub debug_file: DebugFile,
}
pub type DebugFiles = BTreeMap<(String, Option<String>), DebugFile>;

/// Reads the binary file for all the declared symbols
/// and potentially external symbols. Turns it into
/// a map of symbol names, as well as finds all
/// external debug files.
pub fn index_symbol_map<'db>(
    db: &'db dyn Db,
    binary: Binary,
) -> anyhow::Result<(DebugFiles, SymbolIndex)> {
    let mut debug_files = DebugFiles::new();

    // load the binary file
    let binary_file = binary.file(db);
    let loaded_file = match load(db, binary_file) {
        Ok(file) => file,
        Err(e) => {
            return Err(e.clone()).with_context(|| {
                format!(
                    "Failed to load binary file: {}",
                    binary_file.path(db).to_string()
                )
            });
        }
    };

    // create debug file for teh binary
    let debug_file = DebugFile::new(db, binary_file, false);
    debug_files.insert((binary_file.path(db).to_string(), None), debug_file.clone());

    // index the symbols in the binary
    let mut symbol_index = SymbolIndex::default();
    symbol_index.index_binary(&loaded_file.object, debug_file)?;

    // next, if we have any mapped objects (via Mach-O)
    // then we'll locate all the debug files, and index their symbols
    let mut indexed_object_files = vec![];
    let object_map = loaded_file.object.object_map();
    for object_file in object_map.objects() {
        let object_path = object_file.path();
        let Ok(object_path) = String::from_utf8(object_path.to_vec()) else {
            tracing::debug!("Failed to parse object file path: {:?}", object_file.path());
            continue;
        };
        let Ok(member) = object_file
            .member()
            .map(|m| String::from_utf8(m.to_vec()))
            .transpose()
        else {
            tracing::debug!(
                "Failed to parse object file member: {:?}",
                object_file.member()
            );
            continue;
        };

        let file = match File::build(db, object_path.clone(), member.clone()) {
            Ok(file) => file,
            Err(e) => {
                db.report_critical(format!(
                    "Failed to load debug file {object_path} with member: {member:?}: {e}",
                ));
                continue;
            }
        };
        // Create a debug file for this object
        let debug_file = DebugFile::new(db, file, true);
        debug_files.insert((object_path, member), debug_file.clone());
        indexed_object_files.push(debug_file);
    }

    // split objects by index
    let grouped_symbols = object_map
        .symbols()
        .iter()
        .into_group_map_by(|s| s.object_index());

    for (object_index, symbols) in grouped_symbols {
        let debug_file = indexed_object_files[object_index as usize].clone();
        symbol_index.index_mapped_file(symbols.into_iter(), debug_file)?;
    }

    // finally, sort all functions by address
    symbol_index
        .functions_by_address
        .sort_unstable_by_key(|s| s.address);

    Ok((debug_files, symbol_index))
}

/// Fast symbol-based index built from symbol tables
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SymbolIndex {
    /// Function name -> [module1::func -> info, module2::func -> info, ...]
    /// Grouped by lookup_name, then by full SymbolName
    pub functions: BTreeMap<String, BTreeMap<SymbolName, Symbol>>,

    /// Non-function symbols, grouped similarly
    pub symbols: BTreeMap<String, BTreeMap<SymbolName, Symbol>>,

    /// All functions sorted by address for binary search lookup
    /// Used for address-to-function mapping
    pub functions_by_address: Vec<Symbol>,
}

impl SymbolIndex {
    /// Find function by exact name match
    pub fn get_function(&self, name: &SymbolName) -> Option<&Symbol> {
        self.functions.get(&name.lookup_name)?.get(name)
    }

    /// Find all functions with the given lookup name
    pub fn get_functions_by_lookup_name(
        &self,
        lookup_name: &str,
    ) -> Option<&BTreeMap<SymbolName, Symbol>> {
        self.functions.get(lookup_name)
    }

    /// Find function containing the given address using binary search
    pub fn function_at_address(&self, address: u64) -> Option<&Symbol> {
        // Binary search to find the function with the highest address <= target
        match self
            .functions_by_address
            .binary_search_by_key(&address, |f| f.address)
        {
            Ok(idx) => Some(&self.functions_by_address[idx]),
            Err(idx) => {
                // binary_search returns the insertion point when not found
                // We want the function just before this point
                if idx > 0 {
                    Some(&self.functions_by_address[idx - 1])
                } else {
                    None
                }
            }
        }
    }

    pub fn index_binary(&mut self, object: &object::File<'_>, debug_file: DebugFile) -> Result<()> {
        for s in object.symbols() {
            let Ok(name) = s.name_bytes() else {
                tracing::debug!("Failed to parse symbol name at: {:#016x}", s.address());
                continue;
            };

            let symbol = RawSymbol::new(name.to_vec());

            let Ok(demangled) = symbol.demangle() else {
                tracing::debug!("Failed to demangle symbol at : {:#016x}", s.address());
                continue;
            };

            // We'll assume that symbols from the .TEXT section are
            // functions
            let is_function = s.kind() == object::SymbolKind::Text;

            let index_name = demangled.lookup_name.clone();
            let entry = Symbol {
                name: demangled.clone(),
                address: s.address(),
                debug_file,
            };
            let map = if is_function {
                self.functions_by_address.push(entry.clone());
                &mut self.functions
            } else {
                &mut self.symbols
            };

            // Insert the symbol into the appropriate map
            map.entry(index_name)
                .or_default()
                .insert(demangled.clone(), entry);
        }
        Ok(())
    }

    pub fn index_mapped_file<'a>(
        &mut self,
        symbol_iter: impl Iterator<Item = &'a object::ObjectMapEntry<'a>>,
        debug_file: DebugFile,
    ) -> Result<()> {
        for s in symbol_iter {
            let symbol = RawSymbol::new(s.name().to_vec());

            let Ok(demangled) = symbol.demangle() else {
                tracing::debug!("Failed to demangle symbol at : {:#016x}", s.address());
                continue;
            };

            // We'll assume that symbols all have 0 size
            // and functions are non-zero.
            let is_function = s.size() > 0;

            let index_name = demangled.lookup_name.clone();
            let entry = Symbol {
                name: demangled.clone(),
                address: s.address(),
                debug_file,
            };
            let map = if is_function {
                self.functions_by_address.push(entry.clone());
                &mut self.functions
            } else {
                &mut self.symbols
            };

            // Insert the symbol into the appropriate map
            map.entry(index_name)
                .or_default()
                .insert(demangled.clone(), entry);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::env::current_exe;

    use crate::{DebugDb, DebugInfo};
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_symbol_index_basic() -> Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        let db = DebugDb::new();
        let exe_path = "bin/test_binaries/small";
        let debug_info = DebugInfo::new(&db, exe_path).expect("Failed to load debug info");

        // Build the symbol index
        let (_debug_files, symbol_index) = index_symbol_map(&db, debug_info.binary).unwrap();

        // Verify we have some functions indexed
        assert!(
            !symbol_index.functions.is_empty(),
            "Should have indexed some functions"
        );
        assert!(
            !symbol_index.functions_by_address.is_empty(),
            "Should have functions sorted by address"
        );

        // Verify functions are sorted by address
        let addresses: Vec<u64> = symbol_index
            .functions_by_address
            .iter()
            .map(|f| f.address)
            .collect();
        let mut sorted_addresses = addresses.clone();
        sorted_addresses.sort();
        assert_eq!(
            addresses, sorted_addresses,
            "Functions should be sorted by address"
        );

        tracing::info!(
            "Symbol index created successfully with {} function groups and {} total functions",
            symbol_index.functions.len(),
            symbol_index.functions_by_address.len()
        );

        // Test address lookup
        if let Some(first_func) = symbol_index.functions_by_address.first() {
            let found_func = symbol_index.function_at_address(first_func.address);
            assert!(
                found_func.is_some(),
                "Should find function at its own address"
            );
            assert_eq!(found_func.unwrap().address, first_func.address);
        }

        Ok(())
    }

    #[test]
    fn test_symbol_index_performance() -> Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        let db = DebugDb::new();
        let exe_path = current_exe().unwrap();
        let exe_path = exe_path.to_str().unwrap();
        let binary = db
            .analyze_file(exe_path)
            .expect("Failed to analyze binary file")
            .0;
        let start = std::time::Instant::now();
        let (_debug_files, symbol_index) = index_symbol_map(&db, binary).unwrap();
        let symbol_index_time = start.elapsed();

        tracing::info!(
            "Symbol index built in {:?}. Got: {} functions and {} symbols",
            symbol_index_time,
            symbol_index.functions.len(),
            symbol_index.symbols.len()
        );

        // This should be much faster than full DWARF indexing
        // We expect it to be under 100ms for most binaries
        assert!(
            symbol_index_time.as_millis() < 5000,
            "Symbol index should be fast, took {:?}",
            symbol_index_time
        );

        Ok(())
    }
}
