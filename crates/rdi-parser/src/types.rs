//! Type parsing using unsynn

use itertools::Itertools;
use std::{fmt, sync::Arc};

use ::rust_types::*;
use unsynn::*;

// Define some types
unsynn! {
    keyword As = "as";
    keyword Const = "const";
    keyword Dyn = "dyn";
    keyword FnKw = "fn";
    keyword For = "for";
    keyword Impl = "impl";
    keyword Mut = "mut";
    keyword Str = "str";
    keyword Unsafe = "unsafe";
    type Amp = PunctAny<'&'>;

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

    #[derive(Clone)]
    pub struct PtrType {
        pointer_type: Cons<PunctAny<'*'>, Either<Const, Mut>>,
        pub inner: Box<Type>,
    }
    #[derive(Clone)]
    pub struct RefType {
        amp: Amp,
        mutability: Optional<Mut>,
        pub inner: Box<Type>,
    }



    #[derive(Clone)]
    pub enum ArraySize {
        Fixed(usize),
        Dynamic(Type),
    }


    #[derive(Clone)]
    struct ArrayInner {
        inner: Box<Type>,
        size: Optional<Cons<PunctAny<';'>, ArraySize>>,
    }

    #[derive(Clone)]
    pub struct Array {
        inner: BracketGroupContaining<ArrayInner>,
    }

    #[derive(Clone)]
    pub enum Tuple {
        Arity0(ParenthesisGroupContaining<Nothing>),
        // NOTE: the `,` here should not actually be optional, but it
        // seems like rustc outputs these incorrectly
        Arity1(ParenthesisGroupContaining<Cons<Box<Type>, Optional<PunctAny<','>>>>),
        ArityN(
            ParenthesisGroupContaining<
                Cons<
                    AtLeast<1, Cons<Box<Type>, PunctAny<','>>>,
                    Cons<Box<Type>, Optional<PunctAny<','>>>
                >
            >,
        )
    }

    #[derive(Clone)]
    struct FnResult {
        arrow: Cons<PunctJoint<'-'>, PunctAny<'>'>>,
        ret: Type,
    }

    #[derive(Clone)]
    pub struct FnType {
        unsafe_kw: Optional<Unsafe>,
        fn_kw: FnKw,
        args: ParenthesisGroupContaining<DelimitedVec<Type, PunctAny<','>>>,
        ret: Optional<FnResult>,
    }

    #[derive(Clone)]
    pub struct Slice {
        _amp: Amp,
        inner: BracketGroupContaining<Box<Type>>,
    }

    #[derive(Clone)]
    pub struct StrSlice {
        _amp: Amp,
        inner: Str,
    }


    #[derive(Clone)]
    pub enum Type {
        Slice(Slice),
        StrSlice(StrSlice),
        Ref(RefType),
        Never(PunctAny<'!'>),
        Array(Array),
        Function(FnType),
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

    pub fn concrete_size(&self) -> Option<usize> {
        self.inner.content.size.0.first().and_then(|c| {
            match &c.value.second {
                ArraySize::Fixed(size) => Some(*size),
                ArraySize::Dynamic(_) => None, // Dynamic size is not concrete
            }
        })
    }
    pub fn generic_size(&self) -> Option<&Type> {
        self.inner
            .content
            .size
            .0
            .first()
            .and_then(|c| match &c.value.second {
                ArraySize::Fixed(_) => None,
                ArraySize::Dynamic(t) => Some(t),
            })
    }
}

impl Tuple {
    pub fn inner(&self) -> Vec<Type> {
        match self {
            Tuple::Arity0(_) => vec![],
            Tuple::Arity1(inner) => vec![*inner.content.first.clone()],
            Tuple::ArityN(inner) => inner
                .content
                .first
                .0
                .iter()
                .map(|c| *c.value.clone().first.clone())
                .chain(std::iter::once(*inner.content.second.first.clone()))
                .collect(),
        }
    }
}

impl FnType {
    pub fn args(&self) -> Vec<Type> {
        self.args
            .content
            .0
            .iter()
            .map(|t| t.value.clone())
            .collect()
    }

    pub fn ret(&self) -> Option<&Type> {
        self.ret.0.first().map(|c| &c.value.ret)
    }
}

impl fmt::Display for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}", self.inner())?;
        if let Some(size) = self.concrete_size() {
            write!(f, "; {}", size)?;
        }
        if let Some(size) = self.generic_size() {
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

impl PtrType {
    pub fn is_mutable(&self) -> bool {
        matches!(self.pointer_type.second, Either::Second(Mut(_)))
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

impl fmt::Display for FnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fn")?;
        if !self.args.content.0.is_empty() {
            write!(
                f,
                "({})",
                self.args.content.0.iter().map(|t| &t.value).join(", ")
            )?;
        } else {
            write!(f, "()")?;
        }
        if let Some(ret) = &self.ret.0.first() {
            write!(f, " -> {}", ret.value.ret)?;
        }
        Ok(())
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
            Type::Slice(s) => write!(f, "&[{}]", s.inner.content),
            Type::StrSlice(_) => write!(f, "&str"),
            Type::Array(a) => write!(f, "{a}"),
            Type::DynTrait(d) => write!(f, "{d}"),
            Type::Tuple(t) => {
                let elements = t.inner();
                if elements.len() == 1 {
                    write!(f, "({},)", elements[0])
                } else {
                    write!(f, "({})", elements.iter().map(|e| e.to_string()).join(", "))
                }
            }
            Type::Ptr(p) => write!(f, "{}{}", p.pointer_type.tokens_to_string(), p.inner),
            Type::Path(p) => write!(f, "{p}"),
            Type::Function(fn_type) => write!(f, "{fn_type}"),
            Type::Never(_) => write!(f, "!"),
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

pub type ParsedSymbol = (Vec<String>, String, Option<String>);

/// A simpler parsing approach for symbols
///
/// All we truly care about is splitting it into:
///
/// - the module path prefix
/// - the type name
/// - the hash (if present)
///
/// e.g. `core::num::nonzero::NonZero<u8>::ilog2::hc1106854ed63a858`
/// would be parsed into:
/// - `["core", "num", "nonzero", "NonZero<u8>"]`
/// - `ilog2`
/// - `Some("hc1106854ed63a858")`
///
/// We can do that without incurring the parsing overhead of the full
/// `Path` and `Type` parsers, which are more complex and handle
/// more cases than we need here.
pub fn parse_symbol(s: &str) -> anyhow::Result<ParsedSymbol> {
    // First, we need to split the string by `::` while respecting angle brackets
    let mut segments = Vec::with_capacity(4);
    let mut current_segment = String::with_capacity(64);
    let mut angle_depth = 0;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                angle_depth += 1;
                current_segment.push(ch);
            }
            '>' => {
                angle_depth -= 1;
                current_segment.push(ch);
            }
            ':' if angle_depth == 0 && chars.peek() == Some(&':') => {
                // We found `::` at the top level
                chars.next(); // consume the second ':'
                if !current_segment.is_empty() {
                    segments.push(current_segment.trim().to_string());
                    current_segment.clear();
                }
            }
            '\n' | '\r' | '\t' | ' ' => {
                // Ignore consecutive whitespace characters
                // and replace with a single space character
                if !current_segment.is_empty() {
                    if !current_segment.ends_with(' ') {
                        current_segment.push(' ');
                    }
                }
            }
            _ => {
                current_segment.push(ch);
            }
        }
    }

    // Don't forget the last segment
    if !current_segment.is_empty() {
        segments.push(current_segment.trim().to_string());
    }

    if segments.is_empty() {
        anyhow::bail!("Empty symbol path");
    }

    // Now we need to identify the hash, function name, and module path
    let hash = if let Some(last) = segments.last() {
        if last.starts_with('h') && last.chars().skip(1).all(|c| c.is_ascii_hexdigit()) {
            segments.pop()
        } else {
            None
        }
    } else {
        None
    };

    let Some(function_name) = segments.pop() else {
        anyhow::bail!("No function name found");
    };

    segments.shrink_to_fit();
    let module_path = segments;

    Ok((module_path, function_name, hash))
}

pub fn parse_type(s: &str) -> unsynn::Result<Type> {
    let mut iter = s.to_token_iter();
    let ty = Cons::<Type, EndOfStream>::parse(&mut iter)?;
    Ok(ty.first)
}

impl Type {
    pub fn as_typedef(&self) -> TypeDef {
        match self {
            Type::Path(path) => {
                // Convert path to typedef
                path.as_typedef()
            }
            Type::Slice(slice) => {
                let inner = slice.inner.content.clone().as_typedef();
                TypeDef::Primitive(PrimitiveDef::Slice(SliceDef {
                    element_type: Arc::new(inner),
                    data_ptr_offset: 0,
                    length_offset: 0,
                }))
            }
            Type::StrSlice(_) => {
                // For str slices, we can just return a Str type
                TypeDef::Primitive(PrimitiveDef::StrSlice(StrSliceDef {
                    data_ptr_offset: 0,
                    length_offset: 0,
                }))
            }
            Type::Ref(ref_type) => {
                let inner = ref_type.inner.as_typedef();
                TypeDef::Primitive(PrimitiveDef::Reference(ReferenceDef {
                    mutable: ref_type.is_mutable(),
                    pointed_type: Arc::new(inner),
                }))
            }
            Type::Ptr(ptr_type) => {
                let inner = ptr_type.inner.as_typedef();
                TypeDef::Primitive(PrimitiveDef::Pointer(PointerDef {
                    mutable: ptr_type.is_mutable(),
                    pointed_type: Arc::new(inner),
                }))
            }
            Type::Array(array) => {
                let inner = array.inner().clone().as_typedef();
                let length = if let Some(size_type) = array.concrete_size() {
                    // Try to extract numeric literal from the size
                    // For now, we'll default to 0 if we can't parse it
                    size_type.to_string().parse::<usize>().unwrap_or(0)
                } else {
                    0 // Unknown size
                };

                TypeDef::Primitive(PrimitiveDef::Array(ArrayDef {
                    element_type: Arc::new(inner),
                    length,
                }))
            }
            Type::Tuple(tuple) => {
                let elements: Vec<_> = tuple
                    .inner()
                    .iter()
                    .map(|t| (0, Arc::new(t.as_typedef())))
                    .collect();

                // 0-arity tuple is a unit
                if elements.is_empty() {
                    TypeDef::Primitive(PrimitiveDef::Unit(UnitDef))
                } else {
                    TypeDef::Primitive(PrimitiveDef::Tuple(TupleDef {
                        elements,
                        size: 0, // Would need to calculate from DWARF
                    }))
                }
            }
            Type::DynTrait(_dyn_trait) => {
                // For trait objects, we'll use Other for now
                TypeDef::Other {
                    name: self.to_string(),
                }
            }
            Type::Function(fn_type) => TypeDef::Primitive(PrimitiveDef::Function(FunctionDef {
                return_type: Arc::new(
                    fn_type
                        .ret()
                        .map_or(PrimitiveDef::Unit(UnitDef).into(), |r| r.as_typedef()),
                ),
                arg_types: fn_type
                    .args()
                    .iter()
                    .map(|a| Arc::new(a.as_typedef()))
                    .collect(),
            })),
            Type::Never(_) => {
                // For the never type, we can just return a unit type
                TypeDef::Primitive(PrimitiveDef::Never(()))
            }
        }
    }
}

impl Path {
    fn as_typedef(&self) -> TypeDef {
        // First, let's extract the segments
        let segments = self.segments();
        if segments.is_empty() {
            return TypeDef::Other {
                name: String::new(),
            };
        }

        // Get the last segment as the base type name
        let last_segment = segments.last().unwrap();

        // Check if this is a primitive type
        if segments.len() == 1 {
            match last_segment.as_str() {
                "u8" => {
                    return UnsignedIntDef { size: 1 }.into();
                }
                "u16" => {
                    return UnsignedIntDef { size: 2 }.into();
                }
                "u32" => {
                    return UnsignedIntDef { size: 4 }.into();
                }
                "u64" => {
                    return UnsignedIntDef { size: 8 }.into();
                }
                "u128" => {
                    return UnsignedIntDef { size: 16 }.into();
                }
                "usize" => {
                    return UnsignedIntDef {
                        size: std::mem::size_of::<usize>(),
                    }
                    .into();
                }
                "i8" => return IntDef { size: 1 }.into(),
                "i16" => return IntDef { size: 2 }.into(),
                "i32" => return IntDef { size: 4 }.into(),
                "i64" => return IntDef { size: 8 }.into(),
                "i128" => return IntDef { size: 16 }.into(),
                "isize" => {
                    return IntDef {
                        size: std::mem::size_of::<isize>(),
                    }
                    .into();
                }
                "f32" => return FloatDef { size: 4 }.into(),
                "f64" => return FloatDef { size: 8 }.into(),
                "bool" => return TypeDef::Primitive(PrimitiveDef::Bool(())),
                "char" => return TypeDef::Primitive(PrimitiveDef::Char(())),
                "str" => return TypeDef::Primitive(PrimitiveDef::Str(())),
                "()" => return TypeDef::Primitive(PrimitiveDef::Unit(UnitDef)),
                _ => {}
            }
        }

        // Check if this is a standard library type by examining the path
        let is_std = segments[0] == "std" || segments[0] == "core" || segments[0] == "alloc";
        let is_hashbrown = segments[0] == "hashbrown";

        tracing::trace!("Parser segments: {:?}, is_std: {}", segments, is_std);

        if is_std || is_hashbrown || segments.len() == 1 {
            // Parse the last segment for generic types
            // (we're guaranteed to have at least one segment here)
            if let Some(path_segment) = self.segments.0.last() {
                if let PathSegment::Segment(segment) = &path_segment.value {
                    let type_name = segment.ident.to_string();

                    let get_generics = || {
                        segment.generics.0.first().map_or_else(
                            || vec![],
                            |generic_args| match &generic_args.value {
                                GenericArgs::Parsed { inner, .. } => {
                                    inner.0.iter().map(|d| d.value.clone()).collect()
                                }
                                GenericArgs::Unparsed(_) => vec![],
                            },
                        )
                    };

                    tracing::trace!("Checking std type: '{}' against known types", type_name);

                    match type_name.as_str() {
                        "String" => {
                            tracing::trace!("Matched String type!");
                            return TypeDef::Std(StdDef::String(StringDef(VecDef {
                                length_offset: 0,
                                data_ptr_offset: 0,
                                inner_type: Arc::new(TypeDef::Primitive(
                                    PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }),
                                )),
                            })));
                        }
                        "Vec" => {
                            let inner = get_generics()
                                .first()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            return TypeDef::Std(StdDef::Vec(VecDef {
                                inner_type: inner,
                                length_offset: 0,
                                data_ptr_offset: 0,
                            }));
                        }
                        "Option" => {
                            let inner = get_generics()
                                .first()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            return TypeDef::Std(StdDef::Option(OptionDef {
                                name: "Option".to_string(),
                                discriminant: Discriminant {
                                    offset: 0,
                                    ty: DiscriminantType::Implicit,
                                },
                                some_offset: 0,
                                some_type: inner,
                                size: 0,
                            }));
                        }
                        "Result" => {
                            let mut generics_iter = get_generics().into_iter();
                            let ok_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            let err_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            return TypeDef::Std(StdDef::Result(ResultDef {
                                name: "Result".to_string(),
                                discriminant: Discriminant {
                                    offset: 0,
                                    ty: DiscriminantType::Implicit,
                                },
                                ok_type,
                                err_type,
                                size: 0,
                            }));
                        }
                        "HashMap" | "BTreeMap" => {
                            let mut generics_iter = get_generics().into_iter();
                            let key_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            let value_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            let variant = match type_name.as_str() {
                                "HashMap" => MapVariant::HashMap {
                                    bucket_mask_offset: 0,
                                    ctrl_offset: 0,
                                    items_offset: 0,
                                    pair_size: 0,
                                    key_offset: 0,
                                    value_offset: 0,
                                },
                                "BTreeMap" => MapVariant::BTreeMap {
                                    length_offset: 0,
                                    root_offset: 0,
                                    root_layout: BTreeRootLayout {
                                        node_offset: 0,
                                        height_offset: 0,
                                    },
                                    node_layout: BTreeNodeLayout {
                                        keys_offset: 0,
                                        vals_offset: 0,
                                        len_offset: 0,
                                        edges_offset: 0,
                                    },
                                },
                                _ => unreachable!(),
                            };
                            tracing::trace!("Matched Map type: '{type_name}'");
                            return TypeDef::Std(StdDef::Map(MapDef {
                                key_type,
                                value_type,
                                variant,
                            }));
                        }
                        "Box" | "Rc" | "Arc" | "Cell" | "RefCell" | "UnsafeCell" | "Mutex"
                        | "RwLock" => {
                            let inner = get_generics()
                                .into_iter()
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef::Other {
                                        name: "Unknown".to_string(),
                                    })
                                });
                            let variant = match type_name.as_str() {
                                "Box" => SmartPtrVariant::Box,
                                "Rc" => SmartPtrVariant::Rc,
                                "Arc" => SmartPtrVariant::Arc,
                                "Cell" => SmartPtrVariant::Cell,
                                "RefCell" => SmartPtrVariant::RefCell,
                                "UnsafeCell" => SmartPtrVariant::UnsafeCell,
                                "Mutex" => SmartPtrVariant::Mutex,
                                "RwLock" => SmartPtrVariant::RwLock,
                                _ => unreachable!(),
                            };
                            return TypeDef::Std(StdDef::SmartPtr(SmartPtrDef {
                                inner_type: inner,
                                inner_ptr_offset: 0,
                                data_ptr_offset: 0,
                                variant,
                            }));
                        }
                        _ => {}
                    }
                }
            }
        }

        // Default case: treat as a custom type (struct/enum) or alias
        TypeDef::Other {
            name: last_segment.clone(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    use ::rust_types::*;

    use super::*;

    #[track_caller]
    fn parse_symbol(s: &str) -> ParsedSymbol {
        match super::parse_symbol(s) {
            Ok(s) => s,
            Err(e) => {
                panic!(
                    "Failed to parse symbol `{s}`: {e}\nTokens:\n{}",
                    s.to_token_iter().map(|t| format!("{t:?}")).join("\n")
                );
            }
        }
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

    #[allow(unused)]
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
    fn test_symbol_parsing() {
        parse_symbol("u8");
        let mut iter = "<impl foo as bar>".to_token_iter();
        AngleTokenTree::parse(&mut iter).unwrap();
        // let mut iter = "NonZero<u8>".to_token_iter();
        // Cons::<Ident, BracketGroupContaining<Path>>::parse(&mut iter).unwrap();
        parse_symbol("NonZero");
        parse_symbol("NonZero<u8>");
        parse_symbol("core::num::nonzero::NonZero");
        parse_symbol("core::num::nonzero::NonZero<u8>");
        parse_symbol("core::num::nonzero::NonZero<u8>::ilog2::hc1106854ed63a858");
        parse_symbol(
            "drop_in_place<std::backtrace_rs::symbolize::gimli::parse_running_mmaps::MapsEntry>",
        );
        parse_symbol(
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
            parse_symbol(
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
            ,
            (
                vec![
                    "alloc".to_string(),
                    "ffi".to_string(),
                    "c_str".to_string(),
                    "< impl core::convert::From< &core::ffi::c_str::CStr > for alloc::boxed::Box< core::ffi::c_str::CStr > >".to_string(),
                ],
                "from".to_string(),
                Some("hec874816052de6db".to_string())
            )
        );
        parse_symbol("core::ops::function::FnOnce::call_once{{vtable.shim}}::h7689c9dccb951788");

        // other cases
        parse_symbol("_Unwind_SetIP@GCC_3.0");
        parse_symbol("__rustc[95feac21a9532783]::__rust_alloc_zeroed");
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
        parse_type("&&i32");
        parse_type("!");
    }

    #[test]
    fn test_type_printing() {
        let s = "hashbrown::map::HashMap<alloc::string::String, i32, std::hash::random::RandomState, alloc::alloc::Global>";
        assert_eq!(parse_type(s).to_string(), s.to_string());
    }

    #[track_caller]
    fn infer<T: Into<TypeDef> + fmt::Debug>(s: &str, expected: T) {
        let ty = parse_type(s).as_typedef();
        assert_eq!(ty, expected.into(), "Failed to parse type `{s}`");
    }

    fn string_def() -> TypeDef {
        TypeDef::Std(StdDef::String(StringDef(VecDef {
            length_offset: 0,
            data_ptr_offset: 0,
            inner_type: Arc::new(TypeDef::Primitive(PrimitiveDef::UnsignedInt(
                UnsignedIntDef { size: 1 },
            ))),
        })))
    }

    #[test]
    fn test_type_inference() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        infer("u8", UnsignedIntDef::u8());
        infer("u32", UnsignedIntDef::u32());
        infer("()", PrimitiveDef::from(UnitDef));
        infer(
            "(u8,)",
            PrimitiveDef::Tuple(TupleDef {
                elements: vec![(0, Arc::new(UnsignedIntDef::u8().into()))],
                size: 0, // Would need to calculate from DWARF
            }),
        );
        infer(
            "(u8,u64)",
            PrimitiveDef::Tuple(TupleDef {
                elements: vec![
                    (0, Arc::new(UnsignedIntDef::u8().into())),
                    (0, Arc::new(UnsignedIntDef::u64().into())),
                ],
                size: 0, // Would need to calculate from DWARF
            }),
        );
        infer("&u8", ReferenceDef::new_immutable(UnsignedIntDef::u8()));
        infer("&mut u8", ReferenceDef::new_mutable(UnsignedIntDef::u8()));
        infer(
            "dyn core::fmt::Debug",
            TypeDef::Other {
                name: "dyn core::fmt::Debug".to_string(),
            },
        );
        infer("alloc::vec::Vec<u8>", VecDef::new(UnsignedIntDef::u8()));
        infer(
            "alloc::vec::Vec<alloc::vec::Vec<u8>>",
            VecDef::new(VecDef::new(UnsignedIntDef::u8())),
        );
        infer(
            "alloc::vec::Vec<u8, alloc::alloc::Global>",
            VecDef::new(UnsignedIntDef::u8()),
        );
        infer(
            "core::option::Option<i32>",
            StdDef::Option(OptionDef {
                name: "Option".to_string(),
                discriminant: Discriminant {
                    offset: 0,
                    ty: DiscriminantType::Implicit,
                },
                some_offset: 0,
                some_type: Arc::new(IntDef::i32().into()),
                size: 0,
            }),
        );
        infer(
            "alloc::boxed::Box<i32>",
            SmartPtrDef {
                inner_type: Arc::new(IntDef::i32().into()),
                variant: SmartPtrVariant::Box,
                inner_ptr_offset: 0,
                data_ptr_offset: 0,
            },
        );
        infer("alloc::String::String", string_def());
        infer(
            "std::collections::hash::map::HashMap<alloc::string::String, alloc::string::String>",
            MapDef {
                key_type: Arc::new(string_def()),
                value_type: Arc::new(string_def()),
                variant: MapVariant::HashMap {
                    bucket_mask_offset: 0,
                    ctrl_offset: 0,
                    items_offset: 0,
                    pair_size: 0,
                    key_offset: 0,
                    value_offset: 0,
                },
            },
        );

        infer(
            "core::num::nonzero::NonZero<u8>",
            TypeDef::Other {
                name: "NonZero<u8>".to_string(),
            },
        );

        infer(
            "fn(&u64, &mut core::fmt::Formatter) -> core::result::Result<(), core::fmt::Error>",
            TypeDef::Primitive(PrimitiveDef::Function(FunctionDef {
                arg_types: vec![
                    Arc::new(ReferenceDef::new_immutable(UnsignedIntDef::u64()).into()),
                    Arc::new(
                        ReferenceDef::new_mutable(TypeDef::Other {
                            name: "Formatter".to_string(),
                        })
                        .into(),
                    ),
                ],
                return_type: Arc::new(
                    StdDef::Result(ResultDef {
                        name: "Result".to_string(),
                        discriminant: Discriminant {
                            offset: 0,
                            ty: DiscriminantType::Implicit,
                        },
                        ok_type: Arc::new(TypeDef::Primitive(PrimitiveDef::Unit(UnitDef))),
                        err_type: Arc::new(TypeDef::Other {
                            name: "Error".to_string(),
                        }),
                        size: 0,
                    })
                    .into(),
                ),
            })),
        );
        infer(
            "&[u8]",
            TypeDef::Primitive(PrimitiveDef::Slice(SliceDef {
                element_type: Arc::new(UnsignedIntDef::u8().into()),
                data_ptr_offset: 0,
                length_offset: 0,
            })),
        );
        infer(
            "&str",
            TypeDef::Primitive(PrimitiveDef::StrSlice(StrSliceDef {
                data_ptr_offset: 0,
                length_offset: 0,
            })),
        )
    }

    #[test]
    fn test_symbol_parsing_basic() {
        // Test basic function without generics
        let (module_path, function_name, hash) = parse_symbol("core::num::ilog2::h12345");
        assert_eq!(module_path, vec!["core", "num"]);
        assert_eq!(function_name, "ilog2");
        assert_eq!(hash, Some("h12345".to_string()));

        // Test function with generics in module path
        let (module_path, function_name, hash) =
            parse_symbol("core::num::nonzero::NonZero<u8>::ilog2::hc1106854ed63a858");
        assert_eq!(module_path, vec!["core", "num", "nonzero", "NonZero<u8>"]);
        assert_eq!(function_name, "ilog2");
        assert_eq!(hash, Some("hc1106854ed63a858".to_string()));

        // Test function without hash
        let (module_path, function_name, hash) =
            parse_symbol("std::collections::HashMap<String, i32>::insert");
        assert_eq!(
            module_path,
            vec!["std", "collections", "HashMap<String, i32>"]
        );
        assert_eq!(function_name, "insert");
        assert_eq!(hash, None);

        // Test nested generics
        let (module_path, function_name, hash) =
            parse_symbol("std::collections::HashMap<String, Vec<i32>>::get");
        assert_eq!(
            module_path,
            vec!["std", "collections", "HashMap<String, Vec<i32>>"]
        );
        assert_eq!(function_name, "get");
        assert_eq!(hash, None);

        // Test single segment (just function name)
        let (module_path, function_name, hash) = parse_symbol("main");
        assert_eq!(module_path, Vec::<String>::new());
        assert_eq!(function_name, "main");
        assert_eq!(hash, None);

        // Test single segment with hash
        let (module_path, function_name, hash) = parse_symbol("main::h123abc");
        assert_eq!(module_path, Vec::<String>::new());
        assert_eq!(function_name, "main");
        assert_eq!(hash, Some("h123abc".to_string()));
    }

    #[test]
    fn test_symbol_parsing_complex_cases() {
        // Test from the original parser tests
        let (module_path, function_name, hash) = parse_symbol(
            "alloc::ffi::c_str::<impl core::convert::From<&core::ffi::c_str::CStr> for alloc::boxed::Box<core::ffi::c_str::CStr>>::from::hec874816052de6db",
        );

        assert_eq!(
            module_path,
            vec![
                "alloc",
                "ffi",
                "c_str",
                "<impl core::convert::From<&core::ffi::c_str::CStr> for alloc::boxed::Box<core::ffi::c_str::CStr>>"
            ]
        );
        assert_eq!(function_name, "from");
        assert_eq!(hash, Some("hec874816052de6db".to_string()));
    }

    #[test]
    fn test_symbol_parsing_errors() {
        // Test empty string
        assert!(super::parse_symbol("").is_err());

        // Test only hash
        assert!(super::parse_symbol("h123abc").is_err());
    }
}
