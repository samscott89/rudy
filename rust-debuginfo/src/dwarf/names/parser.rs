use std::fmt;

use itertools::Itertools;
use unsynn::*;

use crate::typedef::TypeDef;

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

impl Type {
    pub fn as_typedef(&self) -> TypeDef {
        use crate::typedef::{
            ArrayDef, DefKind, MapDef, MapVariant, OptionDef, PointerDef, PrimitiveDef,
            ReferenceDef, ResultDef, SmartPtrDef, SmartPtrVariant, StdDef, StringDef, TupleDef,
            TypeRef, VecDef,
        };
        use std::sync::Arc;

        match self {
            Type::Path(path) => {
                // Convert path to typedef
                path.as_typedef()
            }
            Type::Ref(ref_type) => {
                let inner = ref_type.inner.as_typedef();
                TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Reference(ReferenceDef {
                        mutable: ref_type.is_mutable(),
                        pointed_type: Arc::new(inner),
                    })),
                }
            }
            Type::Ptr(ptr_type) => {
                let inner = ptr_type.inner.as_typedef();
                TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Pointer(PointerDef {
                        mutable: ptr_type.is_mutable(),
                        pointed_type: Arc::new(inner),
                    })),
                }
            }
            Type::Array(array) => {
                let inner = array.inner().clone().as_typedef();
                let length = if let Some(size_type) = array.size() {
                    // Try to extract numeric literal from the size
                    // For now, we'll default to 0 if we can't parse it
                    size_type.to_string().parse::<usize>().unwrap_or(0)
                } else {
                    0 // Unknown size
                };

                TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Array(ArrayDef {
                        element_type: Arc::new(inner),
                        length,
                    })),
                }
            }
            Type::Tuple(tuple) => {
                let elements: Vec<Arc<TypeDef>> = tuple
                    .inner
                    .content
                    .0
                    .iter()
                    .map(|t| Arc::new(t.value.as_typedef()))
                    .collect();

                TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Tuple(TupleDef {
                        element_types: elements,
                        alignment: 0, // Would need to calculate from DWARF
                        size: 0,      // Would need to calculate from DWARF
                    })),
                }
            }
            Type::DynTrait(_dyn_trait) => {
                // For trait objects, we'll use Other for now
                TypeDef {
                    kind: DefKind::Other {
                        name: self.to_string(),
                    },
                }
            }
        }
    }
}

impl Path {
    fn as_typedef(&self) -> TypeDef {
        use crate::typedef::{
            DefKind, FloatDef, IntDef, MapDef, MapVariant, OptionDef, PrimitiveDef, ResultDef,
            SmartPtrDef, SmartPtrVariant, StdDef, StringDef, UnitDef, UnsignedIntDef, VecDef,
        };
        use std::sync::Arc;

        // First, let's extract the segments
        let segments = self.segments();
        if segments.is_empty() {
            return TypeDef {
                kind: DefKind::Other {
                    name: String::new(),
                },
            };
        }

        // Get the last segment as the base type name
        let last_segment = segments.last().unwrap();

        // Check if this is a primitive type
        if segments.len() == 1 {
            let unsigned_def = |size| TypeDef {
                kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size })),
            };
            let int_def = |size| TypeDef {
                kind: DefKind::Primitive(PrimitiveDef::Int(IntDef { size })),
            };
            let float_def = |size| TypeDef {
                kind: DefKind::Primitive(PrimitiveDef::Float(FloatDef { size })),
            };
            match last_segment.as_str() {
                "u8" => {
                    return unsigned_def(1);
                }
                "u16" => {
                    return unsigned_def(2);
                }
                "u32" => {
                    return unsigned_def(4);
                }
                "u64" => {
                    return unsigned_def(8);
                }
                "u128" => {
                    return unsigned_def(16);
                }
                "usize" => {
                    return unsigned_def(std::mem::size_of::<usize>());
                }
                "i8" => {
                    return int_def(1);
                }
                "i16" => {
                    return int_def(2);
                }
                "i32" => {
                    return int_def(4);
                }
                "i64" => {
                    return int_def(8);
                }
                "i128" => {
                    return int_def(16);
                }
                "isize" => {
                    return int_def(std::mem::size_of::<isize>());
                }
                "f32" => {
                    return float_def(4);
                }
                "f64" => {
                    return float_def(8);
                }
                "bool" => {
                    return TypeDef {
                        kind: DefKind::Primitive(PrimitiveDef::Bool(())),
                    };
                }
                "char" => {
                    return TypeDef {
                        kind: DefKind::Primitive(PrimitiveDef::Char(())),
                    };
                }
                "()" => {
                    return TypeDef {
                        kind: DefKind::Primitive(PrimitiveDef::Unit(UnitDef)),
                    };
                }
                _ => {}
            }
        }

        // Check if this is a standard library type by examining the path
        let is_std = segments[0] == "std" || segments[0] == "core" || segments[0] == "alloc";

        if is_std {
            // Parse the last segment for generic types
            // (we're guaranteed to have at least one segment here)
            if let Some(path_segment) = self.segments.0.last() {
                if let PathSegment::Segment(segment) = &path_segment.value {
                    let type_name = segment.ident.to_string();

                    // Extract generic arguments if present
                    let generics: Vec<Type> = match segment.generics.0.first() {
                        Some(generic_args) => match &generic_args.value {
                            GenericArgs::Parsed { inner, .. } => {
                                inner.0.iter().map(|d| d.value.clone()).collect()
                            }
                            // we failed to parse the generic args, so we wont be able to use those later.
                            GenericArgs::Unparsed(_) => vec![],
                        },
                        None => vec![],
                    };

                    match type_name.as_str() {
                        "String" => {
                            return TypeDef {
                                kind: DefKind::Std(StdDef::String(StringDef)),
                            };
                        }
                        "Vec" => {
                            let inner = generics
                                .first()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            return TypeDef {
                                kind: DefKind::Std(StdDef::Vec(VecDef { inner_type: inner })),
                            };
                        }
                        "Option" => {
                            let inner = generics
                                .first()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            return TypeDef {
                                kind: DefKind::Std(StdDef::Option(OptionDef { inner_type: inner })),
                            };
                        }
                        "Result" => {
                            let mut generics_iter = generics.into_iter();
                            let ok_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            let err_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            return TypeDef {
                                kind: DefKind::Std(StdDef::Result(ResultDef { ok_type, err_type })),
                            };
                        }
                        "HashMap" | "BTreeMap" => {
                            let mut generics_iter = generics.into_iter();
                            let key_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            let value_type = generics_iter
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            let variant = match type_name.as_str() {
                                "HashMap" => MapVariant::HashMap,
                                "BTreeMap" => MapVariant::BTreeMap,
                                _ => unreachable!(),
                            };
                            return TypeDef {
                                kind: DefKind::Std(StdDef::Map(MapDef {
                                    key_type,
                                    value_type,
                                    variant,
                                    size: 0, // Would need to calculate from DWARF
                                })),
                            };
                        }
                        "Box" | "Rc" | "Arc" | "Cell" | "RefCell" | "Mutex" | "RwLock" => {
                            let inner = generics
                                .into_iter()
                                .next()
                                .map(|t| Arc::new(t.as_typedef()))
                                .unwrap_or_else(|| {
                                    Arc::new(TypeDef {
                                        kind: DefKind::Other {
                                            name: "Unknown".to_string(),
                                        },
                                    })
                                });
                            let variant = match type_name.as_str() {
                                "Box" => SmartPtrVariant::Box,
                                "Rc" => SmartPtrVariant::Rc,
                                "Arc" => SmartPtrVariant::Arc,
                                "Cell" => SmartPtrVariant::Cell,
                                "RefCell" => SmartPtrVariant::RefCell,
                                "Mutex" => SmartPtrVariant::Mutex,
                                "RwLock" => SmartPtrVariant::RwLock,
                                _ => unreachable!(),
                            };
                            return TypeDef {
                                kind: DefKind::Std(StdDef::SmartPtr(SmartPtrDef {
                                    inner_type: inner,
                                    variant,
                                })),
                            };
                        }
                        _ => {}
                    }
                }
            }
        }

        // Default case: treat as a custom type (struct/enum) or alias
        TypeDef {
            kind: DefKind::Other {
                name: self.to_string(),
            },
        }
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

pub fn parse_path(s: &str) -> Result<Path> {
    let mut iter = s.to_token_iter();
    let path = Cons::<Path, EndOfStream>::parse(&mut iter)?;
    Ok(path.first)
}

pub fn parse_type(s: &str) -> Result<Type> {
    let mut iter = s.to_token_iter();
    let ty = Cons::<Type, EndOfStream>::parse(&mut iter)?;
    Ok(ty.first)
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use itertools::Itertools;
    use pretty_assertions::assert_eq;

    use crate::typedef::{
        DefKind, MapDef, OptionDef, PrimitiveDef, SmartPtrDef, SmartPtrVariant, StdDef, StringDef,
        UnsignedIntDef, VecDef,
    };

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

    #[test]
    fn test_type_inference() {
        assert_eq!(
            parse_type("u8").as_typedef().kind,
            DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }))
        );
        assert_eq!(
            parse_type("u32").as_typedef().kind,
            DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 4 }))
        );
        assert_eq!(
            parse_type("&u8").as_typedef().kind,
            DefKind::Primitive(PrimitiveDef::Reference(crate::typedef::ReferenceDef {
                mutable: false,
                pointed_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }))
                })
            }))
        );
        assert_eq!(
            parse_type("&mut u8").as_typedef().kind,
            DefKind::Primitive(PrimitiveDef::Reference(crate::typedef::ReferenceDef {
                mutable: true,
                pointed_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }))
                })
            }))
        );
        assert_eq!(
            parse_type("dyn core::fmt::Debug").as_typedef().kind,
            DefKind::Other {
                name: "dyn core::fmt::Debug".to_string()
            }
        );
        assert_eq!(
            parse_type("alloc::vec::Vec<u8>").as_typedef().kind,
            DefKind::Std(StdDef::Vec(VecDef {
                inner_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }))
                })
            }))
        );
        assert_eq!(
            parse_type("alloc::vec::Vec<alloc::vec::Vec<u8>>")
                .as_typedef()
                .kind,
            DefKind::Std(StdDef::Vec(VecDef {
                inner_type: Arc::new(TypeDef {
                    kind: DefKind::Std(StdDef::Vec(VecDef {
                        inner_type: Arc::new(TypeDef {
                            kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef {
                                size: 1
                            }))
                        })
                    }))
                })
            }))
        );
        assert_eq!(
            parse_type("alloc::vec::Vec<u8, alloc::alloc::Global>")
                .as_typedef()
                .kind,
            DefKind::Std(StdDef::Vec(VecDef {
                inner_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::UnsignedInt(UnsignedIntDef { size: 1 }))
                })
            }))
        );
        assert_eq!(
            parse_type("core::option::Option<i32>").as_typedef().kind,
            DefKind::Std(StdDef::Option(OptionDef {
                inner_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Int(crate::typedef::IntDef { size: 4 }))
                })
            }))
        );
        assert_eq!(
            parse_type("alloc::boxed::Box<i32>").as_typedef().kind,
            DefKind::Std(StdDef::SmartPtr(SmartPtrDef {
                inner_type: Arc::new(TypeDef {
                    kind: DefKind::Primitive(PrimitiveDef::Int(crate::typedef::IntDef { size: 4 }))
                }),
                variant: SmartPtrVariant::Box
            }))
        );
        assert_eq!(
            parse_type("alloc::String::String").as_typedef().kind,
            DefKind::Std(StdDef::String(StringDef))
        );
        assert_eq!(
            parse_type(
                "std::collections::hash::map::HashMap<alloc::string::String, alloc::string::String>"
            )
            .as_typedef()
            .kind,
            DefKind::Std(StdDef::Map(MapDef {
                key_type: Arc::new(TypeDef {
                    kind: DefKind::Std(StdDef::String(StringDef))
                }),
                value_type: Arc::new(TypeDef {
                    kind: DefKind::Std(StdDef::String(StringDef))
                }),
                variant: crate::typedef::MapVariant::HashMap,
                size: 0, // Would need to calculate from DWARF
            }))
        );
        assert_eq!(
            parse_type("core::num::nonzero::NonZero<u8>")
                .as_typedef()
                .kind,
            DefKind::Other {
                name: "core::num::nonzero::NonZero<u8>".to_string()
            }
        );
    }
}
