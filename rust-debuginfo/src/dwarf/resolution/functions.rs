//! Function resolution and metadata extraction

use crate::database::Db;
use crate::dwarf::loader::{Offset, RawDie};
use crate::dwarf::unit::UnitRef;
use crate::dwarf::utils::{get_string_attr, pretty_print_die_entry};
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

pub enum FunctionDeclarationType {
    Closure,
    ClassMethodDeclaration,
    /// Class methods are declared in the class, but implemented elsewhere
    ClassMethodImplementation(Offset),
    Function {
        #[allow(dead_code)]
        inlined: bool,
    },
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
/// NOTE: this is not a subprogram. This would typically appear _inside_
/// a subprogram DIE and is used to track the function that was inlined here.
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
    unit_ref: &UnitRef<'db>,
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

    if let Ok(Some(name)) = get_string_attr(die, gimli::DW_AT_name, &unit_ref) {
        if name.starts_with("{closure#") {
            return FunctionDeclarationType::Closure;
        }
    } else {
        tracing::error!(
            "No name attribute for function at {:#010x}. What is this? {}",
            unit_ref.header.offset().as_debug_info_offset().unwrap().0 + die.offset().0,
            pretty_print_die_entry(die, unit_ref)
        );
    }

    let inlined = matches!(
        die.attr(gimli::DW_AT_inline)
            .ok()
            .flatten()
            .map(|v| v.value()),
        Some(gimli::AttributeValue::Inline(gimli::DW_INL_inlined)),
    );

    FunctionDeclarationType::Function { inlined }
}
