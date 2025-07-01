//! Symbol-based indexing for fast debug info lookups

mod names;

use anyhow::{Context, Result};
use object::{Object, ObjectSymbol};

use crate::file::{load, Binary, DebugFile, File};
use crate::index::FunctionIndex;
use crate::DwarfDb;

pub use names::{RawSymbol, SymbolName, TypeName};
use std::collections::BTreeMap;
use std::path::PathBuf;

use itertools::Itertools;

/// Information about a symbol from the symbol table
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub name: SymbolName,
    pub address: u64,
    pub debug_file: DebugFile,
}
pub type DebugFiles = BTreeMap<(PathBuf, Option<String>), DebugFile>;

/// Reads the binary file for all the declared symbols
/// and potentially external symbols. Turns it into
/// a map of symbol names, as well as finds all
/// external debug files.
pub fn index_symbol_map(
    db: &dyn DwarfDb,
    binary: Binary,
) -> anyhow::Result<(DebugFiles, SymbolIndex)> {
    let mut debug_files = DebugFiles::new();

    // load the binary file
    let binary_file = binary.file(db);
    let loaded_file = match load(db, binary_file) {
        Ok(file) => file,
        Err(e) => {
            return Err(e.clone())
                .with_context(|| format!("Failed to load binary file: {}", binary_file.name(db)));
        }
    };

    // create debug file for teh binary
    let debug_file = DebugFile::new(db, binary_file, false);
    debug_files.insert((debug_file.file(db).path(db).clone(), None), debug_file);

    // index the symbols in the binary (if it has debug info)
    let mut symbol_index = SymbolIndex::default();
    if loaded_file.object.has_debug_symbols() {
        symbol_index.index_binary(&loaded_file.object, debug_file)?;
    }

    // next, if we have any mapped objects (via Mach-O)
    // then we'll locate all the debug files, and index their symbols
    let mut indexed_object_files = vec![None; loaded_file.object.object_map().objects().len()];
    let object_map = loaded_file.object.object_map();
    for (i, object_file) in object_map.objects().iter().enumerate() {
        let object_path = object_file.path();
        let Ok(object_path) = String::from_utf8(object_path.to_vec()) else {
            tracing::debug!("Failed to parse object file path: {:?}", object_file.path());
            continue;
        };
        let object_path = PathBuf::from(object_path);
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
                tracing::error!(
                    "Failed to load debug file {} with member: {member:?}: {e}",
                    object_path.display()
                );
                continue;
            }
        };
        tracing::trace!("Found debug file: {}", file.name(db));
        // Create a debug file for this object
        let debug_file = DebugFile::new(db, file, true);
        debug_files.insert((file.path(db).clone(), member), debug_file);
        indexed_object_files[i] = Some(debug_file);
    }

    // split objects by index
    let grouped_symbols = object_map
        .symbols()
        .iter()
        .into_group_map_by(|s| s.object_index());

    for (object_index, symbols) in grouped_symbols {
        if let Some(debug_file) = indexed_object_files[object_index] {
            tracing::trace!(
                "Indexing mapped symbols for debug file: {}",
                debug_file.name(db)
            );
            symbol_index.index_mapped_file(symbols.into_iter(), debug_file)?;
        }
    }

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

    /// All symbols grouped by file
    ///
    /// This is useful for quickly finding all symbols in a specific file
    pub symbols_by_file: BTreeMap<DebugFile, BTreeMap<RawSymbol, Symbol>>,

    /// All functions sorted by address for binary search lookup
    /// Used for address-to-function mapping
    pub functions_by_address: BTreeMap<u64, Vec<Symbol>>,
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
    pub fn function_at_address(&self, address: u64) -> Option<(u64, &Vec<Symbol>)> {
        // Find the first function(s) with an address less than or equal to the given address
        self.functions_by_address
            .range(..=address)
            .next_back()
            .map(|(base_addr, v)| (*base_addr, v))
    }

    pub fn function_index<'db>(
        &'db self,
        db: &'db dyn DwarfDb,
        debug_file: DebugFile,
    ) -> Option<&'db FunctionIndex<'db>> {
        Some(crate::index::function_index(
            db,
            debug_file,
            self.symbols_by_file.get(&debug_file)?,
        ))
    }

    pub fn index_binary(&mut self, object: &object::File<'_>, debug_file: DebugFile) -> Result<()> {
        let file_symbols = self.symbols_by_file.entry(debug_file).or_default();

        for s in object.symbols() {
            let Ok(name) = s.name_bytes() else {
                tracing::debug!("Failed to parse symbol name at: {:#010x}", s.address());
                continue;
            };

            let symbol = RawSymbol::new(name.to_vec());

            let Ok(demangled) = symbol.demangle() else {
                tracing::trace!(
                    "Failed to demangle symbol at: {:#010x}: {}",
                    s.address(),
                    String::from_utf8_lossy(name)
                );
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
            file_symbols.insert(symbol.clone(), entry.clone());

            let map = if is_function {
                self.functions_by_address
                    .entry(entry.address)
                    .or_default()
                    .push(entry.clone());
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
        let file_symbols = self.symbols_by_file.entry(debug_file).or_default();
        for s in symbol_iter {
            let symbol = RawSymbol::new(s.name().to_vec());

            let Ok(demangled) = symbol.demangle() else {
                tracing::trace!(
                    "Failed to demangle symbol at: {:#010x} {}",
                    s.address(),
                    String::from_utf8_lossy(s.name())
                );
                continue;
            };

            // We'll assume that symbols all have 0 size
            // and functions are non-zero.
            let is_function = s.size() > 0;

            tracing::trace!(
                "Indexing symbol at: {:#010x} {} (is_function: {is_function})",
                s.address(),
                demangled.lookup_name
            );

            let index_name = demangled.lookup_name.clone();
            let entry = Symbol {
                name: demangled.clone(),
                address: s.address(),
                debug_file,
            };
            file_symbols.insert(symbol.clone(), entry.clone());
            let map = if is_function {
                self.functions_by_address
                    .entry(entry.address)
                    .or_default()
                    .push(entry.clone());
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

    use anyhow::Result;

    use super::*;

    #[test]
    fn test_symbol_index_basic() -> Result<()> {
        crate::test_utils::init_tracing();

        // Initialize the debug database and load a binary with debug info
        // test on macos file because it has the external symbol files
        let artifact_dir = crate::test_utils::artifacts_dir(Some("aarch64-apple-darwin"));
        let exe_path = artifact_dir.join("small");

        let db = crate::test_utils::test_db(Some("aarch64-apple-darwin"));
        let db = &db;
        let binary = crate::test_utils::load_binary(db, &exe_path);

        // Build the symbol index
        let (_debug_files, symbol_index) = index_symbol_map(db, binary).unwrap();

        // Verify we have some functions indexed
        assert!(
            !symbol_index.functions.is_empty(),
            "Should have indexed some functions"
        );
        assert!(
            !symbol_index.functions_by_address.is_empty(),
            "Should have functions grouped by address"
        );

        tracing::info!(
            "Symbol index created successfully with {} function groups and {} total functions",
            symbol_index.functions.len(),
            symbol_index.functions_by_address.len()
        );

        // Test address lookup
        if let Some((addr, first_funcs)) = symbol_index.functions_by_address.first_key_value() {
            let (_, found_func) = symbol_index
                .function_at_address(*addr)
                .expect("Should find function at address");
            assert_eq!(found_func, first_funcs);
        }

        Ok(())
    }

    #[test]
    fn test_symbol_index_performance() -> Result<()> {
        crate::test_utils::init_tracing();

        let exe_path = current_exe().unwrap();

        let db = crate::test_utils::test_db(Some("aarch64-apple-darwin"));
        let db = &db;
        let binary = crate::test_utils::load_binary(db, &exe_path);

        let start = std::time::Instant::now();
        let (_debug_files, symbol_index) = index_symbol_map(db, binary).unwrap();
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
            "Symbol index should be fast, took {symbol_index_time:?}"
        );

        Ok(())
    }
}
