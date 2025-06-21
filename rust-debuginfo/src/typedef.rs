//! Definition of types in the Rust language.

use anyhow::Result;
use gimli::UnitSectionOffset;
use std::mem::size_of;
use std::sync::Arc;

use salsa::Update;

use crate::file::DebugFile;

/// Reference to a type in DWARF debug information
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    file: DebugFile,
    cu_offset: UnitSectionOffset<usize>,
    die_offset: usize,
}

impl TypeRef {
    /// Create a TypeRef from a Die
    pub fn from_die<'db>(die: &crate::dwarf::Die<'db>, db: &'db dyn crate::database::Db) -> Self {
        Self {
            file: die.file(db),
            cu_offset: die.cu_offset(db),
            die_offset: die.die_offset(db).0,
        }
    }

    pub fn to_die<'db>(&self, db: &'db dyn crate::database::Db) -> crate::dwarf::Die<'db> {
        crate::dwarf::Die::new(
            db,
            self.file.clone(),
            self.cu_offset,
            gimli::UnitOffset(self.die_offset),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeDef {
    pub kind: DefKind,
}

impl TypeDef {
    pub fn display_name(&self) -> String {
        match &self.kind {
            DefKind::Primitive(primitive_def) => match primitive_def {
                PrimitiveDef::Array(array_def) => format!(
                    "[{}; {}]",
                    array_def.element_type.display_name(),
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
                        .map(|arg| arg.display_name())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "fn({}) -> {}",
                        arg_types,
                        function_def.return_type.display_name()
                    )
                }
                PrimitiveDef::Int(int_def) => {
                    format!("i{}", int_def.size * 8)
                }
                PrimitiveDef::Pointer(pointer_def) => {
                    format!("*{}", pointer_def.pointed_type.display_name())
                }
                PrimitiveDef::Reference(reference_def) => {
                    let mut_ref = if reference_def.mutable { "mut " } else { "" };
                    format!("&{}{}", mut_ref, reference_def.pointed_type.display_name())
                }
                PrimitiveDef::Slice(slice_def) => {
                    format!("&[{}]", slice_def.element_type.display_name())
                }
                PrimitiveDef::Str(_) => "str".to_string(),
                PrimitiveDef::StrSlice(_) => {
                    format!("&str",)
                }
                PrimitiveDef::Tuple(tuple_def) => {
                    let element_types = tuple_def
                        .element_types
                        .iter()
                        .map(|element| element.display_name())
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
                    let inner = smart_ptr_def.inner_type.display_name();
                    match smart_ptr_def.variant {
                        SmartPtrVariant::Box => format!("Box<{}>", inner),
                        _ => format!("{:?}<{}>", smart_ptr_def.variant, inner),
                    }
                }
                StdDef::Map(map_def) => {
                    let key_type = map_def.key_type.display_name();
                    let value_type = map_def.value_type.display_name();
                    format!("{:?}<{}, {}>", map_def.variant, key_type, value_type)
                }
                StdDef::Option(option_def) => {
                    let inner_type = option_def.inner_type.display_name();
                    format!("Option<{}>", inner_type)
                }
                StdDef::Result(result_def) => {
                    let ok_type = result_def.ok_type.display_name();
                    let err_type = result_def.err_type.display_name();
                    format!("Result<{}, {}>", ok_type, err_type)
                }
                StdDef::String(_) => {
                    format!("String")
                }
                StdDef::Vec(vec_def) => {
                    let inner_type = vec_def.inner_type.display_name();
                    format!("Vec<{}>", inner_type)
                }
            },
            DefKind::Struct(struct_def) => struct_def.name.clone(),
            DefKind::Enum(enum_def) => enum_def.name.clone(),
            DefKind::Alias(type_ref) => {
                // For now, just return a placeholder name
                // In a real implementation, you'd resolve this using the TypeRef
                format!(
                    "<alias@{:x}:{:x}>",
                    type_ref.cu_offset.as_debug_info_offset().unwrap().0,
                    type_ref.die_offset
                )
            }
            DefKind::Other { name } => name.to_string(),
        }
    }

    pub fn size(&self) -> Option<usize> {
        match &self.kind {
            DefKind::Primitive(primitive_def) => primitive_def.size(),
            DefKind::Std(std_def) => std_def.size(),
            DefKind::Struct(struct_def) => Some(struct_def.size),
            DefKind::Enum(enum_def) => Some(enum_def.size),
            DefKind::Alias(_type_ref) => {
                // Type resolution would need to happen at a higher level
                None
            }
            DefKind::Other { name: _ } => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum DefKind {
    /// Language-specific primitive types from `core::primitive`
    /// (e.g. `i32`, `f64`, etc.)
    ///
    /// There are all simple types that can
    /// be backed by a single slice of memory
    /// and easily transumted to a Rust type
    Primitive(PrimitiveDef),

    /// Common definitions from the Rust standard library
    Std(StdDef),

    // Custom type definitions
    /// Structs and tuples
    Struct(StructDef),

    /// Enums
    Enum(EnumDef),

    /// Reference to some other type
    ///
    /// We use this when we're traversing a type
    /// definition and want to lazily evaluate nested
    /// types.
    Alias(TypeRef),

    /// Other types not yet supported/handled
    Other { name: String },
}

/// From the Rust standard library:
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum PrimitiveDef {
    Array(ArrayDef),
    Bool(()),
    Char(()),
    Float(FloatDef),
    Function(FunctionDef),
    Int(IntDef),
    Pointer(PointerDef),
    Reference(ReferenceDef),
    Slice(SliceDef),
    /// Technically constructable, `str` is like `[u8; N]`
    /// but where the size is opaque (since utf8 is variable length)
    /// and so rarely seen in the wild. We could have something like `Box<str>`
    /// though.
    Str(()),
    /// A specialization of `Slice` where the referenced type is `str`
    /// Also helps us avoid using the `str` type.
    StrSlice(StrSliceDef),
    Tuple(TupleDef),
    Unit(UnitDef),
    UnsignedInt(UnsignedIntDef),
    // neverExperimental,
}

impl PrimitiveDef {
    fn size(&self) -> Option<usize> {
        let size = match self {
            PrimitiveDef::Array(array_def) => {
                let element_size = array_def.element_type.size()?;
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

        Some(size)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ArrayDef {
    pub element_type: Arc<TypeDef>,
    pub length: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FloatDef {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FunctionDef {
    pub return_type: Arc<TypeDef>,
    pub arg_types: Vec<Arc<TypeDef>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct IntDef {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct PointerDef {
    pub mutable: bool,
    pub pointed_type: Arc<TypeDef>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ReferenceDef {
    /// Is this a mutable reference?
    /// (i.e. `&mut T` vs `&T`)
    pub mutable: bool,

    pub pointed_type: Arc<TypeDef>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SliceDef {
    pub element_type: Arc<TypeDef>,
    pub data_ptr_offset: usize,
    pub length_offset: usize,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StrSliceDef {
    pub data_ptr_offset: usize,
    pub length_offset: usize,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct TupleDef {
    pub element_types: Vec<Arc<TypeDef>>,
    pub alignment: usize,
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct UnitDef;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct UnsignedIntDef {
    /// Size in bytes
    /// (e.g. 1 for u8, 2 for u16, etc.)
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum StdDef {
    SmartPtr(SmartPtrDef),
    Map(MapDef),
    Option(OptionDef),
    Result(ResultDef),
    String(StringDef),
    Vec(VecDef),
}

impl StdDef {
    fn size(&self) -> Option<usize> {
        let size = match self {
            StdDef::SmartPtr(smart_ptr_def) => match smart_ptr_def.variant {
                SmartPtrVariant::Box => size_of::<Box<()>>(),
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
                let res_size = result_def.ok_type.size()?;
                let err_size = result_def.err_type.size()?;
                std::cmp::max(res_size, err_size)
            }
            StdDef::String(_) | StdDef::Vec(_) => size_of::<Vec<()>>(),
        };

        Some(size)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SmartPtrDef {
    pub inner_type: Arc<TypeDef>,
    pub variant: SmartPtrVariant,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum SmartPtrVariant {
    Box,
    Rc,
    Arc,
    RefCell,
    Mutex,
    RwLock,
    Cell,
    UnsafeCell,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct MapDef {
    pub key_type: Arc<TypeDef>,
    pub value_type: Arc<TypeDef>,
    pub variant: MapVariant,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum MapVariant {
    HashMap,
    BTreeMap,
    IndexMap,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct OptionDef {
    pub inner_type: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ResultDef {
    pub ok_type: Arc<TypeDef>,
    pub err_type: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StringDef;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct VecDef {
    pub inner_type: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructDef {
    pub name: String,
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<StructField>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructField {
    pub name: String,
    pub offset: usize,
    pub ty: Arc<TypeDef>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumDef {
    pub name: String,
    pub repr: EnumRepr,
    pub variants: Vec<EnumVariant>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum EnumRepr {
    C,
    Rust,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum EnumVariant {
    Unit(EnumUnitVariant),
    Tuple(EnumTupleVariant),
    Struct(EnumStructVariant),
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumUnitVariant {
    pub name: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumTupleVariant {
    pub name: String,
    pub fields: Vec<StructField>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumStructVariant {
    pub name: String,
    pub fields: Vec<StructField>,
}
