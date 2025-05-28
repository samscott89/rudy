//! Function resolution and metadata extraction

use crate::database::Db;
use crate::dwarf::{DieEntryId, utils::to_range};
use crate::types::FunctionIndexEntry;

/// Function address information
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionAddressInfo {
    /// Base address of the function in the text section
    pub base: u64,
    /// End of the function prologue (if found)
    pub prologue_end: Option<u64>,
    /// end of the function
    pub end: u64,
}

/// Get function address information from a DIE entry
pub fn function_address<'db>(db: &'db dyn Db, die: DieEntryId<'db>) -> Option<FunctionAddressInfo> {
    die.with_entry_and_unit(db, |entry, unit_ref| {
        let ranges = match unit_ref.die_ranges(&entry) {
            Ok(ranges) => ranges,
            Err(e) => {
                db.report_critical(format!("Failed to get ranges: {e}"));
                return None;
            }
        };
        let (start, end) = match to_range(ranges) {
            Ok(Some((start, end))) => (start, end),
            Ok(None) => {
                db.report_critical(format!("No address range found for function"));
                return None;
            }
            Err(e) => {
                db.report_critical(format!("Failed to parse ranges: {e}"));
                return None;
            }
        };

        // attempt to find prologue end with the line program range

        let Some(line_program) = unit_ref.line_program.clone() else {
            return None;
        };

        let mut rows = line_program.clone().rows();
        let mut prologue_end = None;
        while let Some((_, row)) = rows.next_row().ok()? {
            let addr = row.address();
            if addr < start || addr >= end {
                continue;
            }

            // TODO(Sam): deal with non-exact matches?
            if row.prologue_end() {
                tracing::debug!("found prologue end at address {addr:#x}");
                prologue_end = Some(addr);
                break;
            }
        }

        Some(FunctionAddressInfo {
            base: start,
            prologue_end,
            end,
        })
    })
    .flatten()
}

/// Tracked function information
#[salsa::tracked]
pub struct Function<'db> {
    #[return_ref]
    pub name: String,
    #[return_ref]
    pub linkage_name: String,
    pub base_address: u64,
    pub function_body_address: u64,
}

/// Resolve a function index entry to full function information
#[salsa::tracked]
pub fn resolve_function<'db>(db: &'db dyn Db, f: FunctionIndexEntry<'db>) -> Option<Function<'db>> {
    let die = f.die(db);
    let name = die.name(db)?;
    let linkage_name = die.string_attr(db, gimli::DW_AT_linkage_name)?;
    let index = crate::index::index(db);
    let base_address = index
        .data(db)
        .cu_to_base_addr
        .get(&die.cu(db))
        .copied()
        .unwrap_or(0);
    let FunctionAddressInfo {
        base, prologue_end, ..
    } = function_address(db, die)?;
    let body_address = prologue_end.unwrap_or(base);
    Some(Function::new(
        db,
        name,
        linkage_name,
        base_address + base,
        base_address + body_address,
    ))
}
