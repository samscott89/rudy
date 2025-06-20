use std::fmt;

use itertools::Itertools;
use unsynn::{
    BraceGroupContaining, BracketGroupContaining, CommaDelimitedVec, Cons, Delimited, DelimitedVec,
    Either, EndOfStream, Error, Except, Gt, Ident, Lt, Many, Optional, ParenthesisGroupContaining,
    Parse, PathSepDelimitedVec, PunctAlone, PunctAny, PunctJoint, Repeats, ToTokens, TokenCount,
    TokenIter, TokenTree, Transaction, unsynn,
};

// Define some types
unsynn! {
    keyword As = "as";
    keyword Const = "const";
    keyword Dyn = "dyn";
    keyword For = "for";
    keyword Impl = "impl";
    keyword Mut = "mut";
    type Amp = PunctAlone<'&'>;

    /// eats all tokens within two angle brackets
    #[derive(Clone)]
    pub struct AngleTokenTree {
        _lt: Lt,
        // inner can either be another nested AngleTokenTree, or
        // arbitrary non-angled tokens
        inner: Vec<Either<Cons<Except<Either<Lt, Gt>>, TokenTree>, AngleTokenTree>>,
        _gt: Gt,
    }

    #[derive(Clone)]
    pub enum GenericArgs {
        Parsed {
            _lt: Lt,
            inner: CommaDelimitedVec<Type>,
            _gt: Gt,
        },
        // fallback for cases we didn't handle above
        // correctly
        Unparsed(AngleTokenTree)
    }

    // pub struct QualifiedSelf {
    //     lt: Lt,
    //     pub ty_name: Ident,
    //     keyword: As,
    //     // this is a path
    //     pub trait_name: PathSepDelimitedVec<Ident>,
    //     gt: Gt,
    // }

    // pub struct ImplTrait {
    //     impl_kw: Impl,
    //     pub trait_name: Path,
    //     for_kw: For,
    //     pub ty_name: Path,
    // }

    keyword VTable = "vtable";
    keyword Shim = "shim";
    type VTableShim = Cons<VTable, PunctAny<'.'>, Shim>;
    keyword VTableType = "vtable_type";

    #[derive(Clone)]
    pub struct Segment {
        pub ident: Ident,
        generics: Optional<GenericArgs>,
        // for some weirdo cases like `core::ops::function::FnOnce::call_once{{vtable.shim}}`
        vtable_shim: Optional<BraceGroupContaining<BraceGroupContaining<VTableShim>>>,
    }

    #[derive(Clone)]
    enum PathSegment {
        Segment(Segment),
        QualifiedSegment(AngleTokenTree),
        VTableType(BraceGroupContaining<VTableType>),
    }

    #[derive(Clone)]
    pub struct DynTrait {
        dyn_kw: Dyn,
        pub traits: DelimitedVec<Type, PunctAny<'+'>>,
    }

    type ConstPtr = Cons<PunctAny<'*'>, Const>;
    type MutPtr = Cons<PunctAny<'*'>, Mut>;

    #[derive(Clone)]
    pub struct PtrType {
        pointer_type: Either<ConstPtr, MutPtr>,
        pub inner: Box<Type>,
    }
    #[derive(Clone)]
    pub struct RefType {
        amp: Amp,
        mutability: Optional<Mut>,
        pub inner: Box<Type>,
    }

    #[derive(Clone)]
    struct ArrayInner {
        inner: Box<Type>,
        size: Optional<Cons<PunctAny<';'>, Type>>,
    }

    #[derive(Clone)]
    pub struct Array {
        inner: BracketGroupContaining<ArrayInner>,
    }

    #[derive(Clone)]
    pub struct Tuple {
        inner: ParenthesisGroupContaining<DelimitedVec<Type, PunctAny<','>>>,
    }

    #[derive(Clone)]
    pub enum Type {
        Ref(RefType),
        Array(Array),
        DynTrait(DynTrait),
        Tuple(Tuple),
        Ptr(PtrType),
        // note: for some reason it's better if path is last
        Path(Path),
    }

    /// Symbol or binary paths as used in Dwarf information.
    #[derive(Clone)]
    pub struct Path {
        segments: PathSepDelimitedVec<PathSegment>
    }
}

impl Array {
    pub fn inner(&self) -> &Type {
        &self.inner.content.inner
    }

    pub fn size(&self) -> Option<&Type> {
        self.inner.content.size.0.first().map(|c| &c.value.second)
    }
}

impl fmt::Display for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}", self.inner())?;
        if let Some(size) = self.size() {
            write!(f, "; {}", size)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl fmt::Display for DynTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dyn {}",
            self.traits.0.iter().map(|t| &t.value).join(" + ")
        )
    }
}

impl RefType {
    pub fn is_mutable(&self) -> bool {
        self.mutability.0.len() > 0
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.segments().join("::"))
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Ref(r) => {
                write!(f, "&")?;
                if r.is_mutable() {
                    write!(f, "mut ")?;
                }
                write!(f, "{}", r.inner)
            }
            Type::Array(a) => write!(f, "{a}"),
            Type::DynTrait(d) => write!(f, "{d}"),
            Type::Tuple(t) => write!(
                f,
                "({})",
                t.inner.content.0.iter().map(|t| &t.value).join(", ")
            ),
            Type::Ptr(p) => write!(f, "{}{}", p.pointer_type.tokens_to_string(), p.inner),
            Type::Path(p) => write!(f, "{}", p),
        }
    }
}

impl Path {
    pub fn segments(&self) -> Vec<String> {
        self.segments
            .0
            .iter()
            .map(|path_segment| match &path_segment.value {
                PathSegment::Segment(segment) => {
                    format!(
                        "{}{}",
                        segment.ident,
                        match segment.generics.0.first().as_ref().map(|g| &g.value) {
                            Some(GenericArgs::Parsed {
                                _lt: _,
                                inner,
                                _gt: _,
                            }) => {
                                format!(
                                    "<{}>",
                                    inner.0.iter().map(|d| d.value.to_string()).join(", ")
                                )
                            }
                            Some(GenericArgs::Unparsed(angle_token_tree)) => {
                                angle_token_tree.tokens_to_string()
                            }
                            None => String::new(),
                        }
                    )
                }
                p => p.tokens_to_string(),
            })
            .collect()
    }
}

pub fn parse_path(s: &str) -> Result<Path, Error> {
    let mut iter = s.to_token_iter();
    let path = Cons::<Path, EndOfStream>::parse(&mut iter)?;
    Ok(path.first)
}

pub fn parse_type(s: &str) -> Result<Type, Error> {
    let mut iter = s.to_token_iter();
    let ty = Cons::<Type, EndOfStream>::parse(&mut iter)?;
    Ok(ty.first)
}

#[cfg(test)]
mod test {
    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    use super::*;

    #[track_caller]
    fn parse_path(s: &str) -> Path {
        match super::parse_path(s) {
            Ok(p) => p,
            Err(e) => {
                panic!(
                    "Failed to parse path `{s}`: {e}\nTokens:\n{}",
                    s.to_token_iter().map(|t| format!("{t:?}")).join("\n")
                );
            }
        }
    }
    #[track_caller]
    fn parse_path_err(s: &str) {
        let _ = super::parse_path(s).unwrap_err();
    }

    #[track_caller]
    fn parse_type(s: &str) -> Type {
        match super::parse_type(s) {
            Ok(p) => p,
            Err(e) => {
                panic!(
                    "Failed to parse type `{s}`: {e}\nTokens:\n{}",
                    s.to_token_iter().map(|t| format!("{t:?}")).join("\n")
                );
            }
        }
    }

    #[track_caller]
    fn parse_arbitrary<T>(s: &str) -> T
    where
        T: Parse,
    {
        let mut iter = s.to_token_iter();
        match Cons::<T, EndOfStream>::parse(&mut iter) {
            Ok(t) => t.first,
            Err(e) => {
                panic!(
                    "Failed to parse `{s}` as {}: {e}\nTokens:\n{}",
                    std::any::type_name::<T>(),
                    s.to_token_iter().map(|t| format!("{t:?}")).join("\n")
                );
            }
        }
    }

    #[test]
    fn test_path_parsing() {
        parse_path("u8");
        let mut iter = "<impl foo as bar>".to_token_iter();
        AngleTokenTree::parse(&mut iter).unwrap();
        // let mut iter = "NonZero<u8>".to_token_iter();
        // Cons::<Ident, BracketGroupContaining<Path>>::parse(&mut iter).unwrap();
        parse_path("NonZero");
        parse_path("NonZero<u8>");
        parse_path("core::num::nonzero::NonZero");
        parse_path("core::num::nonzero::NonZero<u8>");
        parse_path("core::num::nonzero::NonZero<u8>::ilog2::hc1106854ed63a858");
        parse_path(
            "drop_in_place<std::backtrace_rs::symbolize::gimli::parse_running_mmaps::MapsEntry>",
        );
        parse_path(
            "alloc::ffi::c_str::<
                impl
                core::convert::From<
                    &core::ffi::c_str::CStr
                >
                for
                alloc::boxed::Box<
                    core::ffi::c_str::CStr
                >
            >::from::hec874816052de6db",
        );

        assert_eq!(
            parse_path(
                "alloc::ffi::c_str::<
                    impl
                    core::convert::From<
                        &core::ffi::c_str::CStr
                    >
                    for
                    alloc::boxed::Box<
                        core::ffi::c_str::CStr
                    >
                >::from::hec874816052de6db"
            )
            .segments(),
            vec![
                "alloc".to_string(),
                "ffi".to_string(),
                "c_str".to_string(),
                "< impl core :: convert :: From < & core :: ffi :: c_str :: CStr > for alloc :: boxed :: Box < core :: ffi :: c_str :: CStr > >".to_string(),
                "from".to_string(),
                "hec874816052de6db".to_string(),
            ]
        );
        parse_path("core::ops::function::FnOnce::call_once{{vtable.shim}}::h7689c9dccb951788");

        // unsupported cases

        // libc symbols?
        parse_path_err("_Unwind_SetIP@GCC_3.0");
        // whatever this is
        parse_path_err("__rustc[95feac21a9532783]::__rust_alloc_zeroed");
    }

    #[test]
    fn test_type_parsing() {
        parse_type("u8");
        parse_type("&u8");
        parse_type("dyn core::fmt::Debug");
        parse_type("dyn core::fmt::Debug + core::fmt::Display");
        parse_type("&mut dyn core::fmt::Write");
        parse_type("&[core::fmt::rt::Argument]");
        parse_type("<&alloc::string::String as core::fmt::Debug>::{vtable_type}");
        parse_type("(usize, core::option::Option<usize>)");
        parse_type("*const [i32]");
        parse_type("&mut dyn core::ops::function::FnMut<(usize), Output=bool>");
        // parse_type("");
    }

    #[test]
    fn test_type_printing() {
        let s = "hashbrown::map::HashMap<alloc::string::String, i32, std::hash::random::RandomState, alloc::alloc::Global>";
        assert_eq!(parse_type(s).to_string(), s.to_string());
    }
}
