//! Function resolution and metadata extraction

use crate::database::Db;
use crate::dwarf::FunctionIndexEntry;
use crate::dwarf::loader::{DwarfReader, Offset, RawDie};
use crate::dwarf::unit::UnitRef;
use crate::dwarf::{Die, utils::to_range};
use crate::file::DebugFile;
use crate::types::NameId;

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
pub fn function_address<'db>(db: &'db dyn Db, die: Die<'db>) -> Option<FunctionAddressInfo> {
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
                db.report_critical(format!(
                    "No address range found for function: {}",
                    die.print(db)
                ));
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
#[salsa::tracked(debug)]
pub struct Function<'db> {
    #[returns(ref)]
    pub name: String,
    #[returns(ref)]
    pub linkage_name: String,

    pub relative_address: u64,
    pub relative_body_address: u64,
}

/// Resolve a function index entry to full function information
/// TODO: use both declaration + specification dies?
#[salsa::tracked]
pub fn resolve_function<'db>(db: &'db dyn Db, die: Die<'db>) -> Option<Function<'db>> {
    let name = die.name(db)?;
    let linkage_name = die.string_attr(db, gimli::DW_AT_linkage_name)?;
    let FunctionAddressInfo {
        base, prologue_end, ..
    } = function_address(db, die)?;
    let body_address = prologue_end.unwrap_or(base);
    Some(Function::new(db, name, linkage_name, base, body_address))
}

pub enum FunctionDeclarationType {
    Closure,
    ClassMethodDeclaration,
    ClassMethodImplementation(Offset),
    Function,
    InlinedFunction,
    InlinedFunctionImplementation(Offset),
}

/// Infer what kind of declaration this DIE represents
///
/// Some examples:
///
/// Closure:
///
/// 0x000000c8:         DW_TAG_subprogram
///                      DW_AT_low_pc      (0x0000000000000158)
///                      DW_AT_high_pc     (0x000000000000018c)
///                      DW_AT_frame_base  (DW_OP_reg29 W29)
///                      DW_AT_linkage_name        ("_ZN3std2rt10lang_start28_$u7b$$u7b$closure$u7d$$u7d$17hfe953699872e52f8E")
///                      DW_AT_name        ("{closure#0}<()>")
///                      DW_AT_decl_file   ("/Users/sam/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/rt.rs")
///                      DW_AT_decl_line   (199)
///                      DW_AT_type        (0x00004dda "i32")
///
/// Function with generics:
///
/// 0x00000132:       DW_TAG_subprogram
///                    DW_AT_low_pc        (0x00000000000000f8)
///                    DW_AT_high_pc       (0x0000000000000158)
///                    DW_AT_frame_base    (DW_OP_reg29 W29)
///                    DW_AT_linkage_name  ("_ZN3std2rt10lang_start17h3ee7518cb9a82119E")
///                    DW_AT_name  ("lang_start<()>")
///                    DW_AT_decl_file     ("/Users/sam/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/rt.rs")
///                    DW_AT_decl_line     (192)
///                    DW_AT_type  (0x00009a1c "isize")
///
///
/// Class method:
///
/// 0x000001bc:           DW_TAG_subprogram
///                        DW_AT_linkage_name      ("_ZN3std4hash6random11RandomState3new17he3583681eab89a20E")
///                        DW_AT_name      ("new")
///                        DW_AT_decl_file ("/Users/sam/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/hash/random.rs")
///                        DW_AT_decl_line (56)
///                        DW_AT_type      (0x0000019c "std::hash::random::RandomState")
///                        DW_AT_declaration       (true)
///
/// Implementation of a trait method (with #[inline] annotation:
///
/// 0x000001d1:           DW_TAG_subprogram
///                        DW_AT_linkage_name      ("_ZN73_$LT$std..hash..random..RandomState$u20$as$u20$core..default..Default$GT$7default17h8f7526c79c40ea4cE")
///                        DW_AT_name      ("default")
///                        DW_AT_decl_file ("/Users/sam/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/hash/random.rs")
///                        DW_AT_decl_line (151)
///                        DW_AT_type      (0x0000019c "std::hash::random::RandomState")
///                        DW_AT_inline    (DW_INL_inlined)
///
/// Method that was inlined into another function:
///
/// 0x0000027e:                 DW_TAG_inlined_subroutine
///                              DW_AT_abstract_origin     (0x000064ed "_ZN4core4cell13Cell$LT$T$GT$7replace17hcbb859b11ab45ce0E")
///                              DW_AT_low_pc      (0x00000000000004cc)
///                              DW_AT_high_pc     (0x00000000000004d4)
///                              DW_AT_call_file   ("/Users/sam/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/core/src/cell.rs")
///                              DW_AT_call_line   (429)
///                              DW_AT_call_column (14)
///
/// Specification of a class method:
///
/// 0x0000923c:   DW_TAG_subprogram
///                DW_AT_low_pc    (0x0000000000002cd8)
///                DW_AT_high_pc   (0x0000000000002df4)
///                DW_AT_frame_base        (DW_OP_reg29 W29)
///                DW_AT_specification     (0x000058e0 "_ZN5small11TestStruct08method_017h636bec720e368708E")
pub fn get_declaration_type<'db>(
    _db: &'db dyn Db,
    die: &RawDie<'db>,
    _unit_ref: &UnitRef<'db>,
) -> FunctionDeclarationType {
    if die.attr(gimli::DW_AT_declaration).ok().flatten().is_some() {
        return FunctionDeclarationType::ClassMethodDeclaration;
    }
    if let Some(gimli::AttributeValue::UnitRef(offset)) = die
        .attr(gimli::DW_AT_specification)
        .ok()
        .flatten()
        .map(|v| v.value())
    {
        return FunctionDeclarationType::ClassMethodImplementation(offset);
    }
    if let Some(gimli::AttributeValue::UnitRef(offset)) = die
        .attr(gimli::DW_AT_abstract_origin)
        .ok()
        .flatten()
        .map(|v| v.value())
    {
        return FunctionDeclarationType::InlinedFunctionImplementation(offset);
    }

    if matches!(
        die.attr(gimli::DW_AT_inline)
            .ok()
            .flatten()
            .map(|v| v.value()),
        Some(gimli::AttributeValue::Inline(gimli::DW_INL_inlined)),
    ) {
        return FunctionDeclarationType::InlinedFunction;
    }
    FunctionDeclarationType::Function
}
