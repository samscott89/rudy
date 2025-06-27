//! Definition of types in the Rust language.

use std::mem::size_of;
use std::sync::Arc;

use itertools::Itertools;
use salsa::Update;

/// Reference to a type in DWARF debug information
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    pub cu_offset: usize,
    pub die_offset: usize,
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
                    let elements = tuple_def
                        .elements
                        .iter()
                        .map(|(_, element)| element.display_name())
                        .join(", ");
                    format!("({})", elements)
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
                StdDef::Map(map_def) => map_def.display_name(),
                StdDef::Option(option_def) => {
                    let inner_type = option_def.some_type.display_name();
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
            TypeDef::CEnum(c_enum_def) => c_enum_def.name.clone(),
            TypeDef::Alias(type_ref) => {
                // For now, just return a placeholder name
                // In a real implementation, you'd resolve this using the TypeRef
                format!("<alias@{:x}:{:x}>", type_ref.cu_offset, type_ref.die_offset)
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
            TypeDef::CEnum(c_enum_def) => Some(c_enum_def.size),
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
            (TypeDef::CEnum(e1), TypeDef::CEnum(e2)) => e1.name == e2.name,
            (TypeDef::CEnum(e1), TypeDef::Other { name }) => &e1.name == name,
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

    pub fn dereferenced(&self) -> &TypeDef {
        match self {
            TypeDef::Primitive(PrimitiveDef::Pointer(pointer_def)) => {
                pointer_def.pointed_type.dereferenced()
            }
            TypeDef::Primitive(PrimitiveDef::Reference(reference_def)) => {
                reference_def.pointed_type.dereferenced()
            }
            _ => {
                // If it's not a pointer or reference, return self
                self
            }
        }
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

    /// C-style enumerations (simple named integer constants)
    CEnum(CEnumDef),

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
            PrimitiveDef::Slice(_) => size_of::<&[u8]>(),
            PrimitiveDef::Str(_) => todo!(),
            PrimitiveDef::StrSlice(_) => size_of::<&str>(),
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
                if l.elements.len() != r.elements.len() {
                    return false;
                }
                l.elements
                    .iter()
                    .zip(r.elements.iter())
                    .all(|((_, l), (_, r))| l.matching_type(r))
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
    pub return_type: Arc<TypeDef>,
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

        let mut signature = format!("fn({arg_types})");

        if !matches!(
            self.return_type.as_ref(),
            &TypeDef::Primitive(PrimitiveDef::Unit(_))
        ) {
            signature += format!(" -> {}", self.return_type.display_name()).as_str();
        }
        signature
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
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StrSliceDef {
    pub data_ptr_offset: usize,
    pub length_offset: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct TupleDef {
    /// List of elements + offsets to their data
    pub elements: Vec<(usize, Arc<TypeDef>)>,
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
                MapVariant::HashMap { .. } => size_of::<std::collections::HashMap<(), ()>>(),
                MapVariant::BTreeMap { .. } => size_of::<std::collections::BTreeMap<(), ()>>(),
                MapVariant::IndexMap => todo!(),
            },
            StdDef::Option(def) => def.size,
            StdDef::Result(def) => def.size,
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
            (StdDef::Option(l), StdDef::Option(r)) => l.some_type.matching_type(&r.some_type),
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
}

impl MapDef {
    pub fn display_name(&self) -> String {
        format!(
            "{}<{}, {}>",
            self.variant.name(),
            self.key_type.display_name(),
            self.value_type.display_name()
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct BTreeRootLayout {
    pub node_offset: usize,   // offset to node field within Root
    pub height_offset: usize, // offset to height field within Root
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct BTreeNodeLayout {
    pub keys_offset: usize,  // offset to keys array in LeafNode
    pub vals_offset: usize,  // offset to vals array in LeafNode
    pub len_offset: usize,   // offset to len field in LeafNode
    pub edges_offset: usize, // offset to edges array in InternalNode
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum MapVariant {
    HashMap {
        bucket_mask_offset: usize, // offset within RawTableInner
        ctrl_offset: usize,        // offset to ctrl pointer
        items_offset: usize,       // offset to items count
        pair_size: usize,          // size of a single key-value pair
        key_offset: usize,         // offset to key within a pair
        value_offset: usize,       // offset to value within a pair
    },
    BTreeMap {
        length_offset: usize,         // offset to length field in BTreeMap
        root_offset: usize,           // offset to root field in BTreeMap
        root_layout: BTreeRootLayout, // layout of the root structure
        node_layout: BTreeNodeLayout, // layout of the node structures
    },
    IndexMap,
}

impl MapVariant {
    pub fn name(&self) -> &'static str {
        match self {
            MapVariant::HashMap { .. } => "HashMap",
            MapVariant::BTreeMap { .. } => "BTreeMap",
            MapVariant::IndexMap => "IndexMap",
        }
    }
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
pub struct Discriminant {
    pub ty: DiscriminantType,
    pub offset: usize,
}

impl Discriminant {
    pub fn size(&self) -> usize {
        match &self.ty {
            DiscriminantType::Int(int_def) => int_def.size,
            DiscriminantType::UnsignedInt(unsigned_int_def) => unsigned_int_def.size,
            DiscriminantType::Implicit => 4, // no idea?
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum DiscriminantType {
    Int(IntDef),
    UnsignedInt(UnsignedIntDef),
    Implicit,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumDef {
    pub name: String,
    pub discriminant: Discriminant,
    pub variants: Vec<EnumVariant>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumVariant {
    pub name: String,
    /// The discriminant value for this variant, if known
    ///
    /// If `None`, we should use the index of the variant
    /// in the `variants` vector as the discriminant value.
    pub discriminant: Option<i128>,
    pub layout: Arc<TypeDef>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct OptionDef {
    pub name: String,
    pub discriminant: Discriminant,
    pub some_offset: usize,
    pub some_type: Arc<TypeDef>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ResultDef {
    pub name: String,
    pub discriminant: Discriminant,
    pub ok_type: Arc<TypeDef>,
    pub err_type: Arc<TypeDef>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct CEnumDef {
    pub name: String,
    pub discriminant_type: DiscriminantType,
    pub variants: Vec<CEnumVariant>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct CEnumVariant {
    pub name: String,
    pub value: i128,
}

// Helper functions for common patterns
impl UnsignedIntDef {
    pub fn u8() -> Self {
        Self { size: 1 }
    }
    pub fn u32() -> Self {
        Self { size: 4 }
    }
    pub fn u64() -> Self {
        Self { size: 8 }
    }
}

impl IntDef {
    pub fn i32() -> Self {
        Self { size: 4 }
    }
}

impl ReferenceDef {
    pub fn new_mutable<T: Into<TypeDef>>(pointed_type: T) -> Self {
        Self {
            mutable: true,
            pointed_type: Arc::new(pointed_type.into()),
        }
    }

    pub fn new_immutable<T: Into<TypeDef>>(pointed_type: T) -> Self {
        Self {
            mutable: false,
            pointed_type: Arc::new(pointed_type.into()),
        }
    }
}
