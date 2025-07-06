// ===== TARGETED INDEXING FUNCTIONS =====

use std::collections::BTreeMap;

use itertools::Itertools;

use crate::{
    address::{address_to_location, location_to_address, AddressTree, FunctionAddressInfo},
    die::utils::{get_string_attr, pretty_print_die_entry, to_range},
    file::SourceLocation,
    symbols::{RawSymbol, Symbol},
    visitor::{walk_file, DieVisitor, DieWalker, VisitorNode},
    DebugFile, Die, DwarfDb, SymbolName,
};

#[salsa::tracked(debug)]
pub struct FunctionIndexEntry<'db> {
    #[returns(ref)]
    pub data: FunctionData,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, salsa::Update)]
pub struct FunctionData {
    /// Die entry for the function
    pub declaration_die: Die,
    /// Address range of the function relative to the binary
    pub address_range: Option<(u64, u64)>,
    pub name: String,
    pub specification_die: Option<Die>,
    /// Sometimes we'll find the same definition mulitple times
    /// in the same file due to compilation units
    ///
    /// For now, we'll just store the alternate locations
    /// although we'll probably need to do something else
    pub alternate_locations: Vec<Die>,
}

/// Targeted function index containing only functions
#[salsa::tracked(debug)]
pub struct FunctionIndex<'db> {
    #[returns(ref)]
    pub by_symbol_name: BTreeMap<SymbolName, FunctionIndexEntry<'db>>,
    #[returns(ref)]
    pub by_address: AddressTree,
}

impl<'db> FunctionIndex<'db> {
    pub fn address_to_locations(
        &self,
        db: &dyn DwarfDb,
        address: u64,
    ) -> Vec<(SymbolName, SourceLocation)> {
        self.by_address(db)
            .query_address(address, true)
            .into_iter()
            .filter_map(|f| {
                // we've found a function that contains this address
                // next, shift the address to a relative address
                let relative_address = f.relative_start + (address - f.absolute_start);

                let function = self.by_symbol_name(db).get(&f.name)?.data(db);

                address_to_location(db, relative_address, function).map(|loc| (f.name.clone(), loc))
            })
            .collect()
    }

    pub fn location_to_address(
        &self,
        db: &dyn DwarfDb,
        debug_file: DebugFile,
        location: &SourceLocation,
    ) -> Option<(u64, u64)> {
        location_to_address(db, debug_file, self, location)
    }
}

/// Visitor for building function index efficiently
struct FunctionIndexBuilder<'db> {
    functions: BTreeMap<SymbolName, FunctionIndexEntry<'db>>,
    by_address: Vec<FunctionAddressInfo>,
    symbol_map: &'db BTreeMap<RawSymbol, Symbol>,
}

impl<'db> FunctionIndexBuilder<'db> {
    /// Create a new function index builder
    pub fn new(symbol_map: &'db BTreeMap<RawSymbol, Symbol>) -> Self {
        Self {
            functions: BTreeMap::new(),
            by_address: Vec::new(),
            symbol_map,
        }
    }
}

impl<'db> DieVisitor<'db> for FunctionIndexBuilder<'db> {
    fn visit_die<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        match node.die.tag() {
            gimli::DW_TAG_subprogram => {
                Self::visit_function(walker, node)?;
            }
            gimli::DW_TAG_namespace | gimli::DW_TAG_structure_type => {
                walker.walk_children()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn visit_function<'a>(
        walker: &mut DieWalker<'a, 'db, Self>,
        node: VisitorNode<'a>,
    ) -> anyhow::Result<()> {
        // TODO: Replace all of this with a parser

        let mut linkage_name =
            match get_string_attr(&node.die, gimli::DW_AT_linkage_name, &node.unit_ref) {
                Ok(Some(linkage_name)) => linkage_name,
                Ok(None) => {
                    tracing::trace!(
                        "Skipping function with no linkage name: {}",
                        pretty_print_die_entry(&node.die, &node.unit_ref)
                    );
                    return Ok(());
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to get linkage name for function: {e}: \n{}",
                        pretty_print_die_entry(&node.die, &node.unit_ref)
                    );
                    return Err(e);
                }
            };

        if walker.file.relocatable(walker.db) {
            linkage_name.insert(0, '_'); // Ensure linkage name starts with underscore for relocatable files
        }

        // find if the symbol is actually linked in the binary
        if let Some(symbol) = walker
            .visitor
            .symbol_map
            .get(&RawSymbol::new(linkage_name.as_bytes().to_vec()))
        {
            let address_range = node
                .unit_ref
                .die_ranges(&node.die)
                .map_err(anyhow::Error::from)
                .and_then(to_range)
                .unwrap_or(None);

            let die = walker.get_die(node.die);

            let function_data = FunctionData {
                declaration_die: die,
                address_range,
                name: symbol.name.lookup_name.clone(),
                specification_die: None,
                alternate_locations: vec![],
            };

            // Add to address tree if we have address range
            if let Some((relative_start, relative_end)) = address_range {
                walker.visitor.by_address.push(FunctionAddressInfo {
                    absolute_start: symbol.address,
                    absolute_end: symbol.address + relative_end - relative_start,
                    relative_start,
                    relative_end,
                    name: symbol.name.clone(),
                    file: walker.file,
                });
            } else {
                tracing::trace!(
                    "Function {} has no address range in debug file {}",
                    symbol.name,
                    walker.file.name(walker.db)
                );
            }

            let entry = FunctionIndexEntry::new(walker.db, function_data);
            walker.visitor.functions.insert(symbol.name.clone(), entry);
        } else {
            tracing::trace!(
                "Skipping unlinked function: {linkage_name} in {:#?}",
                walker
                    .visitor
                    .symbol_map
                    .values()
                    .map(|s| &s.name)
                    .join("\n")
            );
        }

        Ok(())
    }
}

/// Index only functions in debug file using visitor pattern
#[salsa::tracked(returns(ref))]
pub fn function_index<'db>(
    db: &'db dyn DwarfDb,
    debug_file: DebugFile,
    symbol_map: &'db BTreeMap<RawSymbol, Symbol>,
) -> FunctionIndex<'db> {
    let start = std::time::Instant::now();
    let mut builder = FunctionIndexBuilder::new(symbol_map);
    if let Err(e) = walk_file(db, debug_file, &mut builder) {
        tracing::error!("Failed to walk debug file: {e}");
    }

    let elapsed = start.elapsed();
    if elapsed.as_secs() > 1 {
        tracing::info!(
            "Indexed {} functions in debug file {} in {}.{:03}s",
            builder.functions.len(),
            debug_file.name(db),
            elapsed.as_secs(),
            elapsed.subsec_millis()
        );
    } else {
        tracing::debug!(
            "Indexed {} functions in debug file {} in {:03}ms",
            builder.functions.len(),
            debug_file.name(db),
            elapsed.as_millis()
        );
    }

    FunctionIndex::new(db, builder.functions, AddressTree::new(builder.by_address))
}
