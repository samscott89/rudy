//! Function resolution and metadata extraction

mod index;
mod variables;

use anyhow::Context as _;
pub use index::{function_index, FunctionData, FunctionIndex, FunctionIndexEntry};
use itertools::Itertools;
use rudy_types::{Layout, PrimitiveLayout, ReferenceLayout};
pub use variables::{resolve_function_variables, Variable};

use crate::{
    die::{
        utils::{get_string_attr, pretty_print_die_entry},
        UnitRef,
    },
    file::RawDie,
    function::variables::variable,
    parser::{
        children::{for_each_child, try_for_each_child},
        combinators::all,
        primitives::{attr, is_member_tag, optional_attr, resolve_type_shallow},
        Parser,
    },
    types::{DieLayout, DieTypeDefinition},
    Die, DwarfDb,
};

type Result<T> = std::result::Result<T, super::Error>;

pub enum FunctionDeclarationType {
    Closure,
    ClassMethodDeclaration,
    /// Class methods are declared in the class, but implemented elsewhere
    ClassMethodImplementation,
    Function {
        #[allow(dead_code)]
        inlined: bool,
    },
    InlinedFunctionImplementation,
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
#[allow(dead_code)]
pub fn get_declaration_type(
    _db: &dyn DwarfDb,
    die: &RawDie,
    unit_ref: &UnitRef,
) -> FunctionDeclarationType {
    if die.attr(gimli::DW_AT_declaration).ok().flatten().is_some() {
        return FunctionDeclarationType::ClassMethodDeclaration;
    }
    if let Some(gimli::AttributeValue::UnitRef(_)) = die
        .attr(gimli::DW_AT_specification)
        .ok()
        .flatten()
        .map(|v| v.value())
    {
        return FunctionDeclarationType::ClassMethodImplementation;
    }
    if let Some(gimli::AttributeValue::DebugInfoRef(_)) = die
        .attr(gimli::DW_AT_specification)
        .ok()
        .flatten()
        .map(|v| v.value())
    {
        return FunctionDeclarationType::ClassMethodImplementation;
    }

    if let Some(gimli::AttributeValue::UnitRef(_)) = die
        .attr(gimli::DW_AT_abstract_origin)
        .ok()
        .flatten()
        .map(|v| v.value())
    {
        return FunctionDeclarationType::InlinedFunctionImplementation;
    }

    if let Ok(Some(name)) = get_string_attr(die, gimli::DW_AT_name, unit_ref) {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct FunctionSignature {
    /// Short name of the function, e.g. "len" for "std::vec::Vec<T>::len"
    pub name: String,

    /// Params for the function, e.g. "self: &mut Self, index: usize"
    pub params: Vec<Variable>,

    /// Return type of the function, e.g. "usize" for "std::vec::Vec<T>::len"
    pub return_type: Option<DieTypeDefinition>,

    /// Somewhat duplicative with the `params` field, this
    /// determines (a) if we have a self parameter, and (b) what kind of self parameter it is.
    pub self_type: Option<SelfType>,
    /// `callable` indicates that we expect this function to be callable.
    /// This is true if the function has a symbol in the binary.
    /// If false, it means this is a synthetic method or a function that cannot be called
    /// (e.g., a trait method without an implementation).
    pub callable: bool,
    /// e.g. /path/to/debug_info.rgcu.o 0x12345
    ///
    /// Mostly useful for debugging
    pub debug_location: String,
}

impl FunctionSignature {
    pub fn new(
        name: String,
        params: Vec<Variable>,
        return_type: Option<DieTypeDefinition>,
        self_type: Option<SelfType>,
        callable: bool,
        debug_location: String,
    ) -> Self {
        Self {
            name,
            params,
            return_type,
            self_type,
            callable,
            debug_location,
        }
    }
}

impl FunctionSignature {
    pub fn print_sig(&self) -> String {
        let params = self
            .params
            .iter()
            .map(|p| {
                format!(
                    "{}: {}",
                    p.name.as_deref().unwrap_or("_"),
                    p.ty.display_name()
                )
            })
            .join(", ");
        if let Some(ret) = &self.return_type {
            format!("fn({params}) -> {}", ret.display_name())
        } else {
            format!("fn({params})")
        }
    }
}

/// Parser to extract function parameters
fn function_parameter() -> impl Parser<Variable> {
    is_member_tag(gimli::DW_TAG_formal_parameter).then(variable())
}

/// Parser to return function declaration information
fn function_declaration() -> impl Parser<(String, Option<DieTypeDefinition>, Vec<Variable>)> {
    all((
        attr::<String>(gimli::DW_AT_name),
        optional_attr::<Die>(gimli::DW_AT_type).then(resolve_type_shallow()),
        // If the child is a formal parameter, then attempt to parse it as a variable
        try_for_each_child(
            is_member_tag(gimli::DW_TAG_formal_parameter)
                .filter()
                .then(function_parameter()),
        ),
    ))
}

/// Parser to extract function specification information
fn function_specification() -> impl Parser<Vec<Variable>> {
    for_each_child(function_parameter())
}

/// Analyze a function to see if it's a method for the target type
#[salsa::tracked]
pub fn resolve_function_signature<'db>(
    db: &'db dyn DwarfDb,
    function_index_entry: FunctionIndexEntry<'db>,
) -> Result<FunctionSignature> {
    let FunctionData {
        declaration_die,
        specification_die,
        alternate_locations,
        ..
    } = function_index_entry.data(db);

    let (name, return_type, parameters) = function_declaration()
        .parse(db, *declaration_die)
        .context("parsing function declaration")?;

    let parameters = if let Some(specification_die) = specification_die {
        // If no parameters in declaration, try to get them from specification
        function_specification()
            .parse(db, *specification_die)
            .context("parsing function specification")?
    } else {
        parameters
    };

    let self_type = if let Some(first_param) = parameters.first() {
        // If the first parameter is self-like, determine its type
        let first_param_name = &first_param.name;
        if matches!(
            first_param_name.as_deref(),
            Some("self" | "&self" | "&mut self")
        ) {
            Some(SelfType::from_param_type(first_param.ty.layout.as_ref()))
        } else {
            None
        }
    } else {
        None
    };

    let mut debug_location = format!("Declaration: {}", declaration_die.location(db));
    if let Some(spec) = specification_die {
        debug_location.push_str(&format!("\nSpecification: {}", spec.location(db)));
    }
    for location in alternate_locations.iter() {
        debug_location.push_str(&format!("\nAlternate: {}", location.location(db)));
    }

    Ok(FunctionSignature::new(
        name,
        parameters,
        return_type,
        self_type,
        true, // callable by default
        debug_location,
    ))
}

/// Type of self parameter in a method
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SelfType {
    /// `self` - takes ownership
    Owned,
    /// `&self` - immutable reference
    Borrowed,
    /// `&mut self` - mutable reference
    BorrowedMut,
}

impl SelfType {
    pub fn from_param_type(param_type: &DieLayout) -> Self {
        match param_type {
            Layout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: true,
                ..
            })) => Self::BorrowedMut,
            Layout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
                mutable: false,
                ..
            })) => Self::Borrowed,
            _ => Self::Owned,
        }
    }
}
