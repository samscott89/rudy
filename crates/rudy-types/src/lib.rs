//! Definition of types in the Rust language.

use std::{mem::size_of, sync::Arc};

use itertools::Itertools;
use salsa::Update;

impl Layout {
    pub fn display_name(&self) -> String {
        match &self {
            Layout::Primitive(primitive_def) => match primitive_def {
                PrimitiveLayout::Array(array_def) => format!(
                    "[{}; {}]",
                    array_def.element_type.display_name(),
                    array_def.length
                ),
                PrimitiveLayout::Bool(_) => "bool".to_string(),
                PrimitiveLayout::Char(_) => "char".to_string(),
                PrimitiveLayout::Float(float_def) => {
                    format!("f{}", float_def.size * 8)
                }
                PrimitiveLayout::Function(function_def) => function_def.display_name(),
                PrimitiveLayout::Int(int_def) => int_def.display_name(),
                PrimitiveLayout::Never(_) => "!".to_string(),
                PrimitiveLayout::Pointer(pointer_def) => {
                    format!("*{}", pointer_def.pointed_type.display_name())
                }
                PrimitiveLayout::Reference(reference_def) => {
                    let mut_ref = if reference_def.mutable { "mut " } else { "" };
                    format!("&{}{}", mut_ref, reference_def.pointed_type.display_name())
                }
                PrimitiveLayout::Slice(slice_def) => {
                    format!("&[{}]", slice_def.element_type.display_name())
                }
                PrimitiveLayout::Str(_) => "str".to_string(),
                PrimitiveLayout::StrSlice(_) => "&str".to_string(),
                PrimitiveLayout::Tuple(tuple_def) => {
                    let elements = tuple_def
                        .elements
                        .iter()
                        .map(|(_, element)| element.display_name())
                        .join(", ");
                    format!("({elements})")
                }
                PrimitiveLayout::Unit(_) => "()".to_string(),
                PrimitiveLayout::UnsignedInt(unsigned_int_def) => unsigned_int_def.display_name(),
            },
            Layout::Std(std_def) => match std_def {
                StdLayout::SmartPtr(smart_ptr_def) => {
                    let inner = smart_ptr_def.inner_type.display_name();
                    match smart_ptr_def.variant {
                        SmartPtrVariant::Box => format!("Box<{inner}>"),
                        _ => format!("{:?}<{}>", smart_ptr_def.variant, inner),
                    }
                }
                StdLayout::Map(map_def) => map_def.display_name(),
                StdLayout::Option(option_def) => {
                    let inner_type = option_def.some_type.display_name();
                    format!("Option<{inner_type}>")
                }
                StdLayout::Result(result_def) => {
                    let ok_type = result_def.ok_type.display_name();
                    let err_type = result_def.err_type.display_name();
                    format!("Result<{ok_type}, {err_type}>")
                }
                StdLayout::String(_) => "String".to_string(),
                StdLayout::Vec(vec_def) => {
                    let inner_type = vec_def.inner_type.display_name();
                    format!("Vec<{inner_type}>")
                }
            },
            Layout::Struct(struct_def) => struct_def.name.clone(),
            Layout::Enum(enum_def) => enum_def.name.clone(),
            Layout::CEnum(c_enum_def) => c_enum_def.name.clone(),
            Layout::Alias { name } => name.to_string(),
        }
    }

    pub fn size(&self) -> Option<usize> {
        match &self {
            Layout::Primitive(primitive_def) => primitive_def.size(),
            Layout::Std(std_def) => std_def.size(),
            Layout::Struct(struct_def) => Some(struct_def.size),
            Layout::Enum(enum_def) => Some(enum_def.size),
            Layout::CEnum(c_enum_def) => Some(c_enum_def.size),
            Layout::Alias { name: _ } => None,
        }
    }

    pub fn matching_type(&self, other: &Layout) -> bool {
        match (self, other) {
            (Layout::Primitive(p1), Layout::Primitive(p2)) => p1.matching_type(p2),
            (Layout::Std(s1), Layout::Std(s2)) => s1.matching_type(s2),
            // TODO: Sam: handle this more robustly
            (Layout::Struct(s1), Layout::Struct(s2)) => s1.name == s2.name,
            (Layout::Struct(s1), Layout::Alias { name }) => &s1.name == name,
            (Layout::Alias { name: left }, Layout::Alias { name: right }) => {
                left.ends_with(right) || right.ends_with(left)
            }
            (Layout::Alias { name }, x) | (x, Layout::Alias { name }) => {
                x.display_name().contains(name)
            }
            // TODO: Sam: handle this more robustly
            (Layout::Enum(e1), Layout::Enum(e2)) => e1.name == e2.name,
            (Layout::CEnum(e1), Layout::CEnum(e2)) => e1.name == e2.name,

            _ => false,
        }
    }

    pub fn as_reference(&self) -> Layout {
        Layout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
            mutable: false,
            pointed_type: Arc::new(self.clone()),
        }))
    }

    pub fn dereferenced(&self) -> &Layout {
        match self {
            Layout::Primitive(PrimitiveLayout::Pointer(pointer_def)) => {
                pointer_def.pointed_type.dereferenced()
            }
            Layout::Primitive(PrimitiveLayout::Reference(reference_def)) => {
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
impl From<PrimitiveLayout> for Layout {
    fn from(primitive: PrimitiveLayout) -> Self {
        Layout::Primitive(primitive)
    }
}

impl From<StdLayout> for Layout {
    fn from(std_def: StdLayout) -> Self {
        Layout::Std(std_def)
    }
}

impl From<StructLayout> for Layout {
    fn from(struct_def: StructLayout) -> Self {
        Layout::Struct(struct_def)
    }
}

impl From<EnumLayout> for Layout {
    fn from(enum_def: EnumLayout) -> Self {
        Layout::Enum(enum_def)
    }
}

// PrimitiveDef conversions
impl From<ArrayLayout> for PrimitiveLayout {
    fn from(array: ArrayLayout) -> Self {
        PrimitiveLayout::Array(array)
    }
}

impl From<FloatLayout> for PrimitiveLayout {
    fn from(float: FloatLayout) -> Self {
        PrimitiveLayout::Float(float)
    }
}

impl From<FunctionLayout> for PrimitiveLayout {
    fn from(function: FunctionLayout) -> Self {
        PrimitiveLayout::Function(function)
    }
}

impl From<IntLayout> for PrimitiveLayout {
    fn from(int: IntLayout) -> Self {
        PrimitiveLayout::Int(int)
    }
}

impl From<PointerLayout> for PrimitiveLayout {
    fn from(pointer: PointerLayout) -> Self {
        PrimitiveLayout::Pointer(pointer)
    }
}

impl From<ReferenceLayout> for PrimitiveLayout {
    fn from(reference: ReferenceLayout) -> Self {
        PrimitiveLayout::Reference(reference)
    }
}

impl From<SliceLayout> for PrimitiveLayout {
    fn from(slice: SliceLayout) -> Self {
        PrimitiveLayout::Slice(slice)
    }
}

impl From<StrSliceLayout> for PrimitiveLayout {
    fn from(str_slice: StrSliceLayout) -> Self {
        PrimitiveLayout::StrSlice(str_slice)
    }
}

impl From<TupleLayout> for PrimitiveLayout {
    fn from(tuple: TupleLayout) -> Self {
        PrimitiveLayout::Tuple(tuple)
    }
}

impl From<UnitLayout> for PrimitiveLayout {
    fn from(unit: UnitLayout) -> Self {
        PrimitiveLayout::Unit(unit)
    }
}

impl From<UnsignedIntLayout> for PrimitiveLayout {
    fn from(uint: UnsignedIntLayout) -> Self {
        PrimitiveLayout::UnsignedInt(uint)
    }
}

// StdDef conversions
impl From<SmartPtrLayout> for StdLayout {
    fn from(smart_ptr: SmartPtrLayout) -> Self {
        StdLayout::SmartPtr(smart_ptr)
    }
}

impl From<MapLayout> for StdLayout {
    fn from(map: MapLayout) -> Self {
        StdLayout::Map(map)
    }
}

impl From<StringLayout> for StdLayout {
    fn from(string: StringLayout) -> Self {
        StdLayout::String(string)
    }
}

impl From<VecLayout> for StdLayout {
    fn from(vec: VecLayout) -> Self {
        StdLayout::Vec(vec)
    }
}

// Convenience constructors for primitives with unit values
impl From<()> for PrimitiveLayout {
    fn from(_: ()) -> Self {
        PrimitiveLayout::Bool(())
    }
}

// Chain conversions for common patterns
impl From<UnsignedIntLayout> for Layout {
    fn from(uint: UnsignedIntLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::UnsignedInt(uint))
    }
}

impl From<IntLayout> for Layout {
    fn from(int: IntLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::Int(int))
    }
}

impl From<FloatLayout> for Layout {
    fn from(float: FloatLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::Float(float))
    }
}

impl From<ReferenceLayout> for Layout {
    fn from(reference: ReferenceLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::Reference(reference))
    }
}

impl From<StringLayout> for Layout {
    fn from(string: StringLayout) -> Self {
        Layout::Std(StdLayout::String(string))
    }
}

impl From<VecLayout> for Layout {
    fn from(vec: VecLayout) -> Self {
        Layout::Std(StdLayout::Vec(vec))
    }
}

impl From<MapLayout> for Layout {
    fn from(map: MapLayout) -> Self {
        Layout::Std(StdLayout::Map(map))
    }
}

impl From<SmartPtrLayout> for Layout {
    fn from(smart_ptr: SmartPtrLayout) -> Self {
        Layout::Std(StdLayout::SmartPtr(smart_ptr))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct TypeDefinition<L = ()>
where
    L: Update,
{
    /// The layout of the type definition
    ///
    /// This is an owned reference to the layout,
    /// so it can be shared across multiple definitions
    pub layout: Arc<Layout>,

    /// The location of the type definition in the DWARF
    /// information. This is used to resolve
    /// the type definition in the debug information.
    pub location: L,
}

impl<L> TypeDefinition<L>
where
    L: Update,
{
    pub fn new(location: L, layout: Layout) -> Self {
        Self {
            layout: Arc::new(layout),
            location,
        }
    }

    pub fn display_name(&self) -> String {
        self.layout.display_name()
    }

    pub fn size(&self) -> Option<usize> {
        self.layout.size()
    }

    pub fn matching_type(&self, other: &Self) -> bool {
        self.layout.matching_type(&other.layout)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum Layout {
    /// Language-specific primitive types from `core::primitive`
    /// (e.g. `i32`, `f64`, etc.)
    ///
    /// There are all simple types that can
    /// be backed by a single slice of memory
    /// and easily transumted to a Rust type
    Primitive(PrimitiveLayout),

    /// Common definitions from the Rust standard library
    Std(StdLayout),

    // Custom type definitions
    /// Structs and tuples
    Struct(StructLayout),

    /// Enums
    Enum(EnumLayout),

    /// C-style enumerations (simple named integer constants)
    CEnum(CEnumLayout),

    /// Type currently known only by its name
    Alias { name: String },
}

/// From the Rust standard library:
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum PrimitiveLayout {
    Array(ArrayLayout),
    Bool(()),
    Char(()),
    Float(FloatLayout),
    Function(FunctionLayout),
    Int(IntLayout),
    Never(()),
    Pointer(PointerLayout),
    Reference(ReferenceLayout),
    Slice(SliceLayout),
    /// Technically constructable, `str` is like `[u8; N]`
    /// but where the size is opaque (since utf8 is variable length)
    /// and so rarely seen in the wild. We could have something like `Box<str>`
    /// though.
    Str(()),
    /// A specialization of `Slice` where the referenced type is `str`
    /// Also helps us avoid using the `str` type.
    StrSlice(StrSliceLayout),
    Tuple(TupleLayout),
    Unit(UnitLayout),
    UnsignedInt(UnsignedIntLayout),
    // neverExperimental,
}

impl PrimitiveLayout {
    fn size(&self) -> Option<usize> {
        let size = match self {
            PrimitiveLayout::Array(array_def) => {
                let element_size = array_def.element_type.size()?;
                element_size * array_def.length
            }
            PrimitiveLayout::Bool(_) => {
                // bool is 1 byte
                1
            }
            PrimitiveLayout::Char(_) => {
                // char is 4 bytes
                4
            }
            PrimitiveLayout::Float(float_def) => float_def.size,
            PrimitiveLayout::Function(_)
            | PrimitiveLayout::Pointer(_)
            | PrimitiveLayout::Reference(_) => {
                // size of a pointer
                size_of::<usize>()
            }
            PrimitiveLayout::Int(int_def) => {
                // size of an int
                int_def.size
            }
            PrimitiveLayout::Never(_) => {
                // never type is 0 bytes
                0
            }
            PrimitiveLayout::Slice(_) => size_of::<&[u8]>(),
            PrimitiveLayout::Str(_) => unimplemented!(),
            PrimitiveLayout::StrSlice(_) => size_of::<&str>(),
            PrimitiveLayout::Tuple(tuple_def) => tuple_def.size,
            PrimitiveLayout::Unit(_) => 0,
            PrimitiveLayout::UnsignedInt(unsigned_int_def) => {
                // size of an unsigned int
                unsigned_int_def.size
            }
        };

        Some(size)
    }

    fn matching_type(&self, other: &Self) -> bool {
        match (self, other) {
            (p1, p2) if p1 == p2 => true,

            (PrimitiveLayout::Pointer(l), PrimitiveLayout::Pointer(r)) => {
                l.pointed_type.matching_type(&r.pointed_type)
            }
            (PrimitiveLayout::Reference(l), PrimitiveLayout::Reference(r)) => {
                l.pointed_type.matching_type(&r.pointed_type)
            }
            (PrimitiveLayout::Slice(l), PrimitiveLayout::Slice(r)) => {
                l.element_type.matching_type(&r.element_type)
            }
            (PrimitiveLayout::Array(l), PrimitiveLayout::Array(r)) => {
                l.element_type.matching_type(&r.element_type)
            }

            (PrimitiveLayout::Tuple(l), PrimitiveLayout::Tuple(r)) => {
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
pub struct ArrayLayout {
    pub element_type: Arc<Layout>,
    pub length: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FloatLayout {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FunctionLayout {
    pub return_type: Arc<Layout>,
    pub arg_types: Vec<Arc<Layout>>,
}

impl FunctionLayout {
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
            &Layout::Primitive(PrimitiveLayout::Unit(_))
        ) {
            signature += format!(" -> {}", self.return_type.display_name()).as_str();
        }
        signature
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct IntLayout {
    pub size: usize,
}

impl IntLayout {
    pub fn display_name(&self) -> String {
        format!("i{}", self.size * 8)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct PointerLayout {
    pub mutable: bool,
    pub pointed_type: Arc<Layout>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ReferenceLayout {
    /// Is this a mutable reference?
    /// (i.e. `&mut T` vs `&T`)
    pub mutable: bool,

    pub pointed_type: Arc<Layout>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SliceLayout {
    pub element_type: Arc<Layout>,
    pub data_ptr_offset: usize,
    pub length_offset: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StrSliceLayout {
    pub data_ptr_offset: usize,
    pub length_offset: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct TupleLayout {
    /// List of elements + offsets to their data
    pub elements: Vec<(usize, Arc<Layout>)>,
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct UnitLayout;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct UnsignedIntLayout {
    /// Size in bytes
    /// (e.g. 1 for u8, 2 for u16, etc.)
    pub size: usize,
}

impl UnsignedIntLayout {
    pub fn display_name(&self) -> String {
        format!("u{}", self.size * 8)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum StdLayout {
    SmartPtr(SmartPtrLayout),
    Map(MapLayout),
    Option(OptionLayout),
    Result(ResultLayout),
    String(StringLayout),
    Vec(VecLayout),
}

impl StdLayout {
    fn size(&self) -> Option<usize> {
        let size = match self {
            StdLayout::SmartPtr(smart_ptr_def) => match smart_ptr_def.variant {
                SmartPtrVariant::Box => size_of::<Box<()>>(),
                SmartPtrVariant::Rc => size_of::<std::rc::Rc<()>>(),
                SmartPtrVariant::Arc => size_of::<std::sync::Arc<()>>(),
                SmartPtrVariant::RefCell => size_of::<std::cell::RefCell<()>>(),
                SmartPtrVariant::Mutex => size_of::<std::sync::Mutex<()>>(),
                SmartPtrVariant::RwLock => size_of::<std::sync::RwLock<()>>(),
                SmartPtrVariant::Cell => size_of::<std::cell::Cell<()>>(),
                SmartPtrVariant::UnsafeCell => size_of::<std::cell::UnsafeCell<()>>(),
            },
            StdLayout::Map(map_def) => match map_def.variant {
                MapVariant::HashMap { .. } => size_of::<std::collections::HashMap<(), ()>>(),
                MapVariant::BTreeMap { .. } => size_of::<std::collections::BTreeMap<(), ()>>(),
                MapVariant::IndexMap => unimplemented!(),
            },
            StdLayout::Option(def) => def.size,
            StdLayout::Result(def) => def.size,
            StdLayout::String(_) | StdLayout::Vec(_) => size_of::<Vec<()>>(),
        };

        Some(size)
    }

    fn matching_type(&self, other: &Self) -> bool {
        match (self, other) {
            (l, r) if l == r => true,
            (StdLayout::SmartPtr(l), StdLayout::SmartPtr(r)) => {
                l.variant == r.variant && l.inner_type.matching_type(&r.inner_type)
            }
            (StdLayout::Map(l), StdLayout::Map(r)) => {
                l.variant == r.variant
                    && l.key_type.matching_type(&r.key_type)
                    && l.value_type.matching_type(&r.value_type)
            }
            (StdLayout::Option(l), StdLayout::Option(r)) => l.some_type.matching_type(&r.some_type),
            (StdLayout::Result(l), StdLayout::Result(r)) => {
                l.ok_type.matching_type(&r.ok_type) && l.err_type.matching_type(&r.err_type)
            }
            (StdLayout::String(_), StdLayout::String(_)) => true,
            (StdLayout::Vec(l), StdLayout::Vec(r)) => l.inner_type.matching_type(&r.inner_type),
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SmartPtrLayout {
    pub inner_type: Arc<Layout>,
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
pub struct MapLayout {
    pub key_type: Arc<Layout>,
    pub value_type: Arc<Layout>,
    pub variant: MapVariant,
}

impl MapLayout {
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
pub struct StringLayout(pub VecLayout);

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct VecLayout {
    pub length_offset: usize,
    pub data_ptr_offset: usize,
    pub inner_type: Arc<Layout>,
}

impl VecLayout {
    pub fn new<T: Into<Layout>>(inner_type: T) -> Self {
        Self {
            length_offset: 0,
            data_ptr_offset: 0,
            inner_type: Arc::new(inner_type.into()),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructLayout {
    pub name: String,
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<StructField>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructField {
    pub name: String,
    pub offset: usize,
    pub ty: Arc<Layout>,
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
    Int(IntLayout),
    UnsignedInt(UnsignedIntLayout),
    Implicit,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumLayout {
    pub name: String,
    pub discriminant: Discriminant,
    pub variants: Vec<EnumVariantLayout>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumVariantLayout {
    pub name: String,
    /// The discriminant value for this variant, if known
    ///
    /// If `None`, we should use the index of the variant
    /// in the `variants` vector as the discriminant value.
    pub discriminant: Option<i128>,
    pub layout: Arc<Layout>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct OptionLayout {
    pub name: String,
    pub discriminant: Discriminant,
    pub some_offset: usize,
    pub some_type: Arc<Layout>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ResultLayout {
    pub name: String,
    pub discriminant: Discriminant,
    pub ok_type: Arc<Layout>,
    pub ok_offset: usize,
    pub err_type: Arc<Layout>,
    pub err_offset: usize,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct CEnumLayout {
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
impl UnsignedIntLayout {
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

impl IntLayout {
    pub fn i32() -> Self {
        Self { size: 4 }
    }
}

impl ReferenceLayout {
    pub fn new_mutable<T: Into<Layout>>(pointed_type: T) -> Self {
        Self {
            mutable: true,
            pointed_type: Arc::new(pointed_type.into()),
        }
    }

    pub fn new_immutable<T: Into<Layout>>(pointed_type: T) -> Self {
        Self {
            mutable: false,
            pointed_type: Arc::new(pointed_type.into()),
        }
    }
}
