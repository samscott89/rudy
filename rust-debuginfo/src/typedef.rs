//! Definition of types in the Rust language.

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

impl TypeDef {
    pub fn display_name(&self) -> String {
        match &self {
            TypeDef::Primitive(primitive_def) => match primitive_def {
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
                PrimitiveDef::Function(function_def) => function_def.display_name(),
                PrimitiveDef::Int(int_def) => int_def.display_name(),
                PrimitiveDef::Never(_) => "!".to_string(),
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
                PrimitiveDef::UnsignedInt(unsigned_int_def) => unsigned_int_def.display_name(),
            },
            TypeDef::Std(std_def) => match std_def {
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
            TypeDef::Struct(struct_def) => struct_def.name.clone(),
            TypeDef::Enum(enum_def) => enum_def.name.clone(),
            TypeDef::Alias(type_ref) => {
                // For now, just return a placeholder name
                // In a real implementation, you'd resolve this using the TypeRef
                format!(
                    "<alias@{:x}:{:x}>",
                    type_ref.cu_offset.as_debug_info_offset().unwrap().0,
                    type_ref.die_offset
                )
            }
            TypeDef::Other { name } => name.to_string(),
        }
    }

    pub fn size(&self) -> Option<usize> {
        match &self {
            TypeDef::Primitive(primitive_def) => primitive_def.size(),
            TypeDef::Std(std_def) => std_def.size(),
            TypeDef::Struct(struct_def) => Some(struct_def.size),
            TypeDef::Enum(enum_def) => Some(enum_def.size),
            TypeDef::Alias(_type_ref) => {
                // Type resolution would need to happen at a higher level
                None
            }
            TypeDef::Other { name: _ } => None,
        }
    }

    pub fn matching_type(&self, other: &TypeDef) -> bool {
        match (self, other) {
            (TypeDef::Primitive(p1), TypeDef::Primitive(p2)) => p1.matching_type(p2),
            (TypeDef::Std(s1), TypeDef::Std(s2)) => s1.matching_type(s2),
            // TODO: Sam: handle this more robustly
            (TypeDef::Struct(s1), TypeDef::Struct(s2)) => s1.name == s2.name,
            (TypeDef::Struct(s1), TypeDef::Other { name }) => &s1.name == name,
            (TypeDef::Other { name: left }, TypeDef::Other { name: right }) => {
                left.ends_with(right) || right.ends_with(left)
            }
            // TODO: Sam: handle this more robustly
            (TypeDef::Enum(e1), TypeDef::Enum(e2)) => e1.name == e2.name,
            (TypeDef::Enum(e1), TypeDef::Other { name }) => &e1.name == name,
            (TypeDef::Alias(_), TypeDef::Alias(_)) => false,
            _ => false,
        }
    }

    pub fn as_reference(&self) -> TypeDef {
        TypeDef::Primitive(PrimitiveDef::Reference(ReferenceDef {
            mutable: false,
            pointed_type: Arc::new(self.clone()),
        }))
    }
}

// Conversion helpers for cleaner test code
impl From<PrimitiveDef> for TypeDef {
    fn from(primitive: PrimitiveDef) -> Self {
        TypeDef::Primitive(primitive)
    }
}

impl From<StdDef> for TypeDef {
    fn from(std_def: StdDef) -> Self {
        TypeDef::Std(std_def)
    }
}

impl From<StructDef> for TypeDef {
    fn from(struct_def: StructDef) -> Self {
        TypeDef::Struct(struct_def)
    }
}

impl From<EnumDef> for TypeDef {
    fn from(enum_def: EnumDef) -> Self {
        TypeDef::Enum(enum_def)
    }
}

impl From<TypeRef> for TypeDef {
    fn from(type_ref: TypeRef) -> Self {
        TypeDef::Alias(type_ref)
    }
}

// PrimitiveDef conversions
impl From<ArrayDef> for PrimitiveDef {
    fn from(array: ArrayDef) -> Self {
        PrimitiveDef::Array(array)
    }
}

impl From<FloatDef> for PrimitiveDef {
    fn from(float: FloatDef) -> Self {
        PrimitiveDef::Float(float)
    }
}

impl From<FunctionDef> for PrimitiveDef {
    fn from(function: FunctionDef) -> Self {
        PrimitiveDef::Function(function)
    }
}

impl From<IntDef> for PrimitiveDef {
    fn from(int: IntDef) -> Self {
        PrimitiveDef::Int(int)
    }
}

impl From<PointerDef> for PrimitiveDef {
    fn from(pointer: PointerDef) -> Self {
        PrimitiveDef::Pointer(pointer)
    }
}

impl From<ReferenceDef> for PrimitiveDef {
    fn from(reference: ReferenceDef) -> Self {
        PrimitiveDef::Reference(reference)
    }
}

impl From<SliceDef> for PrimitiveDef {
    fn from(slice: SliceDef) -> Self {
        PrimitiveDef::Slice(slice)
    }
}

impl From<StrSliceDef> for PrimitiveDef {
    fn from(str_slice: StrSliceDef) -> Self {
        PrimitiveDef::StrSlice(str_slice)
    }
}

impl From<TupleDef> for PrimitiveDef {
    fn from(tuple: TupleDef) -> Self {
        PrimitiveDef::Tuple(tuple)
    }
}

impl From<UnitDef> for PrimitiveDef {
    fn from(unit: UnitDef) -> Self {
        PrimitiveDef::Unit(unit)
    }
}

impl From<UnsignedIntDef> for PrimitiveDef {
    fn from(uint: UnsignedIntDef) -> Self {
        PrimitiveDef::UnsignedInt(uint)
    }
}

// StdDef conversions
impl From<SmartPtrDef> for StdDef {
    fn from(smart_ptr: SmartPtrDef) -> Self {
        StdDef::SmartPtr(smart_ptr)
    }
}

impl From<MapDef> for StdDef {
    fn from(map: MapDef) -> Self {
        StdDef::Map(map)
    }
}

impl From<OptionDef> for StdDef {
    fn from(option: OptionDef) -> Self {
        StdDef::Option(option)
    }
}

impl From<ResultDef> for StdDef {
    fn from(result: ResultDef) -> Self {
        StdDef::Result(result)
    }
}

impl From<StringDef> for StdDef {
    fn from(string: StringDef) -> Self {
        StdDef::String(string)
    }
}

impl From<VecDef> for StdDef {
    fn from(vec: VecDef) -> Self {
        StdDef::Vec(vec)
    }
}

// Convenience constructors for primitives with unit values
impl From<()> for PrimitiveDef {
    fn from(_: ()) -> Self {
        PrimitiveDef::Bool(())
    }
}

// Chain conversions for common patterns
impl From<UnsignedIntDef> for TypeDef {
    fn from(uint: UnsignedIntDef) -> Self {
        TypeDef::Primitive(PrimitiveDef::UnsignedInt(uint))
    }
}

impl From<IntDef> for TypeDef {
    fn from(int: IntDef) -> Self {
        TypeDef::Primitive(PrimitiveDef::Int(int))
    }
}

impl From<FloatDef> for TypeDef {
    fn from(float: FloatDef) -> Self {
        TypeDef::Primitive(PrimitiveDef::Float(float))
    }
}

impl From<ReferenceDef> for TypeDef {
    fn from(reference: ReferenceDef) -> Self {
        TypeDef::Primitive(PrimitiveDef::Reference(reference))
    }
}

impl From<StringDef> for TypeDef {
    fn from(string: StringDef) -> Self {
        TypeDef::Std(StdDef::String(string))
    }
}

impl From<VecDef> for TypeDef {
    fn from(vec: VecDef) -> Self {
        TypeDef::Std(StdDef::Vec(vec))
    }
}

impl From<OptionDef> for TypeDef {
    fn from(option: OptionDef) -> Self {
        TypeDef::Std(StdDef::Option(option))
    }
}

impl From<MapDef> for TypeDef {
    fn from(map: MapDef) -> Self {
        TypeDef::Std(StdDef::Map(map))
    }
}

impl From<SmartPtrDef> for TypeDef {
    fn from(smart_ptr: SmartPtrDef) -> Self {
        TypeDef::Std(StdDef::SmartPtr(smart_ptr))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum TypeDef {
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
    Never(()),
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
            PrimitiveDef::Never(_) => {
                // never type is 0 bytes
                0
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

    fn matching_type(&self, other: &Self) -> bool {
        match (self, other) {
            (p1, p2) if p1 == p2 => true,

            (PrimitiveDef::Pointer(l), PrimitiveDef::Pointer(r)) => {
                l.pointed_type.matching_type(&r.pointed_type)
            }
            (PrimitiveDef::Reference(l), PrimitiveDef::Reference(r)) => {
                l.pointed_type.matching_type(&r.pointed_type)
            }
            (PrimitiveDef::Slice(l), PrimitiveDef::Slice(r)) => {
                l.element_type.matching_type(&r.element_type)
            }
            (PrimitiveDef::Array(l), PrimitiveDef::Array(r)) => {
                l.element_type.matching_type(&r.element_type)
            }

            (PrimitiveDef::Tuple(l), PrimitiveDef::Tuple(r)) => {
                if l.element_types.len() != r.element_types.len() {
                    return false;
                }
                l.element_types
                    .iter()
                    .zip(r.element_types.iter())
                    .all(|(l, r)| l.matching_type(r))
            }
            _ => false,
        }
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
    pub return_type: Option<Arc<TypeDef>>,
    pub arg_types: Vec<Arc<TypeDef>>,
}

impl FunctionDef {
    pub fn display_name(&self) -> String {
        let arg_types = self
            .arg_types
            .iter()
            .map(|arg| arg.display_name())
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(return_type) = &self.return_type {
            format!("fn({}) -> {}", arg_types, return_type.display_name())
        } else {
            format!("fn({})", arg_types)
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct IntDef {
    pub size: usize,
}

impl IntDef {
    pub fn display_name(&self) -> String {
        format!("i{}", self.size * 8)
    }
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

impl UnsignedIntDef {
    pub fn display_name(&self) -> String {
        format!("u{}", self.size * 8)
    }
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

    fn matching_type(&self, other: &Self) -> bool {
        match (self, other) {
            (l, r) if l == r => true,
            (StdDef::SmartPtr(l), StdDef::SmartPtr(r)) => {
                l.variant == r.variant && l.inner_type.matching_type(&r.inner_type)
            }
            (StdDef::Map(l), StdDef::Map(r)) => {
                l.variant == r.variant
                    && l.key_type.matching_type(&r.key_type)
                    && l.value_type.matching_type(&r.value_type)
            }
            (StdDef::Option(l), StdDef::Option(r)) => l.inner_type.matching_type(&r.inner_type),
            (StdDef::Result(l), StdDef::Result(r)) => {
                l.ok_type.matching_type(&r.ok_type) && l.err_type.matching_type(&r.err_type)
            }
            (StdDef::String(_), StdDef::String(_)) => true,
            (StdDef::Vec(l), StdDef::Vec(r)) => l.inner_type.matching_type(&r.inner_type),
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SmartPtrDef {
    pub inner_type: Arc<TypeDef>,
    pub inner_ptr_offset: usize,
    pub data_ptr_offset: usize,
    pub variant: SmartPtrVariant,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update, Copy)]
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

impl SmartPtrVariant {
    pub fn name(&self) -> &'static str {
        match self {
            SmartPtrVariant::Box => "Box",
            SmartPtrVariant::Rc => "Rc",
            SmartPtrVariant::Arc => "Arc",
            SmartPtrVariant::RefCell => "RefCell",
            SmartPtrVariant::Mutex => "Mutex",
            SmartPtrVariant::RwLock => "RwLock",
            SmartPtrVariant::Cell => "Cell",
            SmartPtrVariant::UnsafeCell => "UnsafeCell",
        }
    }
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
    pub discriminant_offset: usize,
    pub some_offset: usize,
    pub inner_type: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ResultDef {
    pub ok_type: Arc<TypeDef>,
    pub err_type: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StringDef(pub VecDef);

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct VecDef {
    pub length_offset: usize,
    pub data_ptr_offset: usize,
    pub inner_type: Arc<TypeDef>,
}

impl VecDef {
    pub fn new<T: Into<TypeDef>>(inner_type: T) -> Self {
        Self {
            length_offset: 0,
            data_ptr_offset: 0,
            inner_type: Arc::new(inner_type.into()),
        }
    }
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
