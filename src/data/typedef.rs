//! Functionality for introspecting Rust data types

use anyhow::Result;
use std::sync::Arc;

use salsa::Update;

use crate::{database::Db, dwarf::get_typedef, types::NameId};

#[salsa::tracked]
pub struct TypeDef<'db> {
    pub name: Option<NameId<'db>>,
    #[return_ref]
    pub kind: DefKind<'db>,
}

impl<'db> TypeDef<'db> {
    pub fn display_name(&self, db: &'db dyn Db) -> String {
        match self.kind(db) {
            DefKind::Primitive(primitive_def) => match primitive_def {
                PrimitiveDef::Array(array_def) => format!(
                    "[{}; {}]",
                    array_def.element_type.display_name(db),
                    array_def.length
                ),
                PrimitiveDef::Bool(_) => "bool".to_string(),
                PrimitiveDef::Char(_) => "char".to_string(),
                PrimitiveDef::Float(float_def) => {
                    format!("f{}", float_def.size * 8)
                }
                PrimitiveDef::Function(function_def) => {
                    let arg_types = function_def
                        .arg_types
                        .iter()
                        .map(|arg| arg.display_name(db))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "fn({}) -> {}",
                        arg_types,
                        function_def.return_type.display_name(db)
                    )
                }
                PrimitiveDef::Int(int_def) => {
                    format!("i{}", int_def.size * 8)
                }
                PrimitiveDef::Pointer(pointer_def) => {
                    format!("*{}", pointer_def.pointed_type.display_name(db))
                }
                PrimitiveDef::Reference(reference_def) => {
                    let mut_ref = if reference_def.mutable { "mut " } else { "" };
                    format!(
                        "&{}{}",
                        mut_ref,
                        reference_def.pointed_type.display_name(db)
                    )
                }
                PrimitiveDef::Slice(slice_def) => {
                    format!("&[{}]", slice_def.element_type.display_name(db))
                }
                PrimitiveDef::Str(_) => "str".to_string(),
                PrimitiveDef::StrSlice(_) => {
                    format!("&str",)
                }
                PrimitiveDef::Tuple(tuple_def) => {
                    let element_types = tuple_def
                        .element_types
                        .iter()
                        .map(|element| element.display_name(db))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({})", element_types)
                }
                PrimitiveDef::Unit(_) => "()".to_string(),
                PrimitiveDef::UnsignedInt(unsigned_int_def) => {
                    format!("u{}", unsigned_int_def.size * 8)
                }
            },
            DefKind::Std(std_def) => match std_def {
                StdDef::SmartPtr(smart_ptr_def) => {
                    let inner = smart_ptr_def.inner_type.display_name(db);
                    format!("{:?}<{}>", smart_ptr_def.variant, inner)
                }
                StdDef::Map(map_def) => {
                    let key_type = map_def.key_type.display_name(db);
                    let value_type = map_def.value_type.display_name(db);
                    format!("{:?}<{}, {}>", map_def.variant, key_type, value_type)
                }
                StdDef::Option(option_def) => {
                    let inner_type = option_def.inner_type.display_name(db);
                    format!("Option<{}>", inner_type)
                }
                StdDef::Result(result_def) => {
                    let ok_type = result_def.ok_type.display_name(db);
                    let err_type = result_def.err_type.display_name(db);
                    format!("Result<{}, {}>", ok_type, err_type)
                }
                StdDef::String(_) => {
                    format!("String")
                }
                StdDef::Vec(vec_def) => {
                    let inner_type = vec_def.inner_type.display_name(db);
                    format!("Vec<{}>", inner_type)
                }
            },
            DefKind::Struct(struct_def) => struct_def.name.clone(),
            DefKind::Enum(enum_def) => enum_def.name.clone(),
            DefKind::Alias(name_id) => {
                if let Ok(Some(def)) = get_typedef(db, *name_id) {
                    def.display_name(db)
                } else {
                    name_id.as_path(db)
                }
            }
            DefKind::Other { name } => name.to_string(),
        }
    }

    pub fn size(&self, db: &'db dyn Db) -> Result<Option<usize>> {
        match self.kind(db) {
            DefKind::Primitive(primitive_def) => primitive_def.size(db),
            DefKind::Std(std_def) => std_def.size(db),
            DefKind::Struct(struct_def) => Ok(Some(struct_def.size)),
            DefKind::Enum(enum_def) => Ok(Some(enum_def.size)),
            DefKind::Alias(name) => {
                if let Some(def) = get_typedef(db, *name)? {
                    def.size(db)
                } else {
                    Ok(None)
                }
            }
            DefKind::Other { name: _ } => Ok(None),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum DefKind<'db> {
    /// Language-specific primitive types from `core::primitive`
    /// (e.g. `i32`, `f64`, etc.)
    ///
    /// There are all simple types that can
    /// be backed by a single slice of memory
    /// and easily transumted to a Rust type
    Primitive(PrimitiveDef<'db>),

    /// Common definitions from the Rust standard library
    Std(StdDef<'db>),

    // Custom type definitions
    /// Structs and tuples
    Struct(StructDef<'db>),

    /// Enums
    Enum(EnumDef<'db>),

    /// Reference to some other type
    ///
    /// We use this when we're traversing a type
    /// definition and want to lazily evaluate nested
    /// types.
    Alias(NameId<'db>),

    /// Other types not yet supported/handled
    Other { name: String },
}

/// From the Rust standard library:
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum PrimitiveDef<'db> {
    Array(ArrayDef<'db>),
    Bool(()),
    Char(()),
    Float(FloatDef),
    Function(FunctionDef<'db>),
    Int(IntDef),
    Pointer(PointerDef<'db>),
    Reference(ReferenceDef<'db>),
    Slice(SliceDef<'db>),
    /// Technically constructable, `str` is like `[u8; N]`
    /// but where the size is opaque (since utf8 is variable length)
    /// and so rarely seen in the wild. We could have something like `Box<str>`
    /// though.
    Str(()),
    /// A specialization of `Slice` where the referenced type is `str`
    /// Also helps us avoid using the `str` type.
    StrSlice(StrSliceDef),
    Tuple(TupleDef<'db>),
    Unit(UnitDef),
    UnsignedInt(UnsignedIntDef),
    // neverExperimental,
}

impl<'db> PrimitiveDef<'db> {
    fn size(&self, db: &'db dyn Db) -> Result<Option<usize>> {
        let size = match self {
            PrimitiveDef::Array(array_def) => {
                let element_size = array_def.element_type.size(db)?;
                let Some(element_size) = element_size else {
                    return Ok(None);
                };
                element_size * array_def.length
            }
            PrimitiveDef::Bool(_) => {
                // bool is 1 byte
                1
            }
            PrimitiveDef::Char(_) => {
                // char is 4 bytes
                4
            }
            PrimitiveDef::Float(float_def) => float_def.size,
            PrimitiveDef::Function(_) | PrimitiveDef::Pointer(_) | PrimitiveDef::Reference(_) => {
                // size of a pointer
                size_of::<usize>()
            }
            PrimitiveDef::Int(int_def) => {
                // size of an int
                int_def.size
            }
            PrimitiveDef::Slice(slice_def) => slice_def.size,
            PrimitiveDef::Str(_) => todo!(),
            PrimitiveDef::StrSlice(str_slice_def) => str_slice_def.size,
            PrimitiveDef::Tuple(tuple_def) => tuple_def.size,
            PrimitiveDef::Unit(_) => 0,
            PrimitiveDef::UnsignedInt(unsigned_int_def) => {
                // size of an unsigned int
                unsigned_int_def.size
            }
        };

        Ok(Some(size))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct ArrayDef<'db> {
    pub element_type: Arc<TypeDef<'db>>,
    pub length: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct FloatDef {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct FunctionDef<'db> {
    pub return_type: Arc<TypeDef<'db>>,
    pub arg_types: Vec<Arc<TypeDef<'db>>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct IntDef {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct PointerDef<'db> {
    pub pointed_type: Arc<TypeDef<'db>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct ReferenceDef<'db> {
    /// Is this a mutable reference?
    /// (i.e. `&mut T` vs `&T`)
    pub mutable: bool,

    pub pointed_type: Arc<TypeDef<'db>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct SliceDef<'db> {
    pub element_type: Arc<TypeDef<'db>>,
    pub data_ptr_offset: usize,
    pub length_offset: usize,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct StrSliceDef {
    pub data_ptr_offset: usize,
    pub length_offset: usize,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct TupleDef<'db> {
    pub element_types: Vec<Arc<TypeDef<'db>>>,
    pub alignment: usize,
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct UnitDef;

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct UnsignedIntDef {
    /// Size in bytes
    /// (e.g. 1 for u8, 2 for u16, etc.)
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum StdDef<'db> {
    SmartPtr(SmartPtrDef<'db>),
    Map(MapDef<'db>),
    Option(OptionDef<'db>),
    Result(ResultDef<'db>),
    String(StringDef),
    Vec(VecDef<'db>),
}

impl<'db> StdDef<'db> {
    fn size(&self, db: &'db dyn Db) -> Result<Option<usize>> {
        let size = match self {
            StdDef::SmartPtr(smart_ptr_def) => match smart_ptr_def.variant {
                SmartPtrVariant::Rc => size_of::<std::rc::Rc<()>>(),
                SmartPtrVariant::Arc => size_of::<std::sync::Arc<()>>(),
                SmartPtrVariant::RefCell => size_of::<std::cell::RefCell<()>>(),
                SmartPtrVariant::Mutex => size_of::<std::sync::Mutex<()>>(),
                SmartPtrVariant::RwLock => size_of::<std::sync::RwLock<()>>(),
                SmartPtrVariant::Cell => size_of::<std::cell::Cell<()>>(),
                SmartPtrVariant::UnsafeCell => size_of::<std::cell::UnsafeCell<()>>(),
            },
            StdDef::Map(map_def) => match map_def.variant {
                MapVariant::HashMap => size_of::<std::collections::HashMap<(), ()>>(),
                MapVariant::BTreeMap => size_of::<std::collections::BTreeMap<(), ()>>(),
                MapVariant::IndexMap => todo!(),
            },
            StdDef::Option(_) => {
                // due to the way Rust handles options, the size of this
                // will just be a single pointer
                debug_assert_eq!(size_of::<usize>(), size_of::<Option<[u8; 16]>>());
                size_of::<usize>()
            }
            StdDef::Result(result_def) => {
                match result_def
                    .ok_type
                    .size(db)?
                    .zip(result_def.err_type.size(db)?)
                {
                    Some((res_size, err_size)) => std::cmp::max(res_size, err_size),
                    None => return Ok(None),
                }
            }
            StdDef::String(_) | StdDef::Vec(_) => size_of::<Vec<()>>(),
        };

        Ok(Some(size))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct SmartPtrDef<'db> {
    pub inner_type: Arc<TypeDef<'db>>,
    pub variant: SmartPtrVariant,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum SmartPtrVariant {
    Rc,
    Arc,
    RefCell,
    Mutex,
    RwLock,
    Cell,
    UnsafeCell,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct MapDef<'db> {
    pub key_type: Arc<TypeDef<'db>>,
    pub value_type: Arc<TypeDef<'db>>,
    pub variant: MapVariant,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum MapVariant {
    HashMap,
    BTreeMap,
    IndexMap,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct OptionDef<'db> {
    pub inner_type: Arc<TypeDef<'db>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct ResultDef<'db> {
    pub ok_type: Arc<TypeDef<'db>>,
    pub err_type: Arc<TypeDef<'db>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct StringDef;

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct VecDef<'db> {
    pub inner_type: Arc<TypeDef<'db>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct StructDef<'db> {
    pub name: String,
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<StructField<'db>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct StructField<'db> {
    pub name: String,
    pub offset: usize,
    pub ty: Arc<TypeDef<'db>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct EnumDef<'db> {
    pub name: String,
    pub repr: EnumRepr,
    pub variants: Vec<EnumVariant<'db>>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum EnumRepr {
    C,
    Rust,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub enum EnumVariant<'db> {
    Unit(EnumUnitVariant),
    Tuple(EnumTupleVariant<'db>),
    Struct(EnumStructVariant<'db>),
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct EnumUnitVariant {
    pub name: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct EnumTupleVariant<'db> {
    pub name: String,
    pub fields: Vec<StructField<'db>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Update)]
pub struct EnumStructVariant<'db> {
    pub name: String,
    pub fields: Vec<StructField<'db>>,
}
