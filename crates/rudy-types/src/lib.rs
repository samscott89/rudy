//! Definition of types in the Rust language.

use std::{mem::size_of, sync::Arc};

use itertools::Itertools;
use salsa::Update;

impl<L: Update> Layout<L> {
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

    pub fn matching_type(&self, other: &Layout<L>) -> bool {
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

    pub fn as_reference(&self, location: L) -> Layout<L>
    where
        Self: Clone,
    {
        Layout::Primitive(PrimitiveLayout::Reference(ReferenceLayout {
            mutable: false,
            pointed_type: Arc::new(TypeDefinition::new(location, self.clone())),
        }))
    }

    pub fn dereferenced(&self) -> &Layout<L> {
        match self {
            Layout::Primitive(PrimitiveLayout::Pointer(pointer_def)) => {
                pointer_def.pointed_type.layout.dereferenced()
            }
            Layout::Primitive(PrimitiveLayout::Reference(reference_def)) => {
                reference_def.pointed_type.layout.dereferenced()
            }
            _ => {
                // If it's not a pointer or reference, return self
                self
            }
        }
    }
}

// Conversion helpers for cleaner test code
impl<L: Update> From<PrimitiveLayout<L>> for Layout<L> {
    fn from(primitive: PrimitiveLayout<L>) -> Self {
        Layout::Primitive(primitive)
    }
}

impl<L: Update> From<StdLayout<L>> for Layout<L> {
    fn from(std_def: StdLayout<L>) -> Self {
        Layout::Std(std_def)
    }
}

impl<L: Update> From<StructLayout<L>> for Layout<L> {
    fn from(struct_def: StructLayout<L>) -> Self {
        Layout::Struct(struct_def)
    }
}

impl<L: Update> From<EnumLayout<L>> for Layout<L> {
    fn from(enum_def: EnumLayout<L>) -> Self {
        Layout::Enum(enum_def)
    }
}

// PrimitiveDef conversions
impl<L: Update> From<ArrayLayout<L>> for PrimitiveLayout<L> {
    fn from(array: ArrayLayout<L>) -> Self {
        PrimitiveLayout::Array(array)
    }
}

impl<L: Update> From<FloatLayout> for PrimitiveLayout<L> {
    fn from(float: FloatLayout) -> Self {
        PrimitiveLayout::Float(float)
    }
}

impl<L: Update> From<FunctionLayout<L>> for PrimitiveLayout<L> {
    fn from(function: FunctionLayout<L>) -> Self {
        PrimitiveLayout::Function(function)
    }
}

impl<L: Update> From<IntLayout> for PrimitiveLayout<L> {
    fn from(int: IntLayout) -> Self {
        PrimitiveLayout::Int(int)
    }
}

impl<L: Update> From<PointerLayout<L>> for PrimitiveLayout<L> {
    fn from(pointer: PointerLayout<L>) -> Self {
        PrimitiveLayout::Pointer(pointer)
    }
}

impl<L: Update> From<ReferenceLayout<L>> for PrimitiveLayout<L> {
    fn from(reference: ReferenceLayout<L>) -> Self {
        PrimitiveLayout::Reference(reference)
    }
}

impl<L: Update> From<SliceLayout<L>> for PrimitiveLayout<L> {
    fn from(slice: SliceLayout<L>) -> Self {
        PrimitiveLayout::Slice(slice)
    }
}

impl<L: Update> From<StrSliceLayout> for PrimitiveLayout<L> {
    fn from(str_slice: StrSliceLayout) -> Self {
        PrimitiveLayout::StrSlice(str_slice)
    }
}

impl<L: Update> From<TupleLayout<L>> for PrimitiveLayout<L> {
    fn from(tuple: TupleLayout<L>) -> Self {
        PrimitiveLayout::Tuple(tuple)
    }
}

impl<L: Update> From<UnitLayout> for PrimitiveLayout<L> {
    fn from(unit: UnitLayout) -> Self {
        PrimitiveLayout::Unit(unit)
    }
}

impl<L: Update> From<UnsignedIntLayout> for PrimitiveLayout<L> {
    fn from(uint: UnsignedIntLayout) -> Self {
        PrimitiveLayout::UnsignedInt(uint)
    }
}

// StdDef conversions
impl<L: Update> From<SmartPtrLayout<L>> for StdLayout<L> {
    fn from(smart_ptr: SmartPtrLayout<L>) -> Self {
        StdLayout::SmartPtr(smart_ptr)
    }
}

impl<L: Update> From<MapLayout<L>> for StdLayout<L> {
    fn from(map: MapLayout<L>) -> Self {
        StdLayout::Map(map)
    }
}

impl<L: Update> From<StringLayout<L>> for StdLayout<L> {
    fn from(string: StringLayout<L>) -> Self {
        StdLayout::String(string)
    }
}

impl<L: Update> From<VecLayout<L>> for StdLayout<L> {
    fn from(vec: VecLayout<L>) -> Self {
        StdLayout::Vec(vec)
    }
}

// Convenience constructors for primitives with unit values
impl<L: Update> From<()> for PrimitiveLayout<L> {
    fn from(_: ()) -> Self {
        PrimitiveLayout::Bool(())
    }
}

// Chain conversions for common patterns
impl<L: Update> From<UnsignedIntLayout> for Layout<L> {
    fn from(uint: UnsignedIntLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::UnsignedInt(uint))
    }
}

impl<L: Update> From<IntLayout> for Layout<L> {
    fn from(int: IntLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::Int(int))
    }
}

impl<L: Update> From<FloatLayout> for Layout<L> {
    fn from(float: FloatLayout) -> Self {
        Layout::Primitive(PrimitiveLayout::Float(float))
    }
}

impl<L: Update> From<ReferenceLayout<L>> for Layout<L> {
    fn from(reference: ReferenceLayout<L>) -> Self {
        Layout::Primitive(PrimitiveLayout::Reference(reference))
    }
}

impl<L: Update> From<StringLayout<L>> for Layout<L> {
    fn from(string: StringLayout<L>) -> Self {
        Layout::Std(StdLayout::String(string))
    }
}

impl<L: Update> From<VecLayout<L>> for Layout<L> {
    fn from(vec: VecLayout<L>) -> Self {
        Layout::Std(StdLayout::Vec(vec))
    }
}

impl<L: Update> From<MapLayout<L>> for Layout<L> {
    fn from(map: MapLayout<L>) -> Self {
        Layout::Std(StdLayout::Map(map))
    }
}

impl<L: Update> From<SmartPtrLayout<L>> for Layout<L> {
    fn from(smart_ptr: SmartPtrLayout<L>) -> Self {
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
    pub layout: Arc<Layout<L>>,

    /// The location of the type definition in the DWARF
    /// information. This is used to resolve
    /// the type definition in the debug information.
    pub location: L,
}

impl<L> TypeDefinition<L>
where
    L: Update,
{
    pub fn new(location: L, layout: Layout<L>) -> Self {
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
pub enum Layout<L = ()>
where
    L: Update,
{
    /// Language-specific primitive types from `core::primitive`
    /// (e.g. `i32`, `f64`, etc.)
    ///
    /// There are all simple types that can
    /// be backed by a single slice of memory
    /// and easily transumted to a Rust type
    Primitive(PrimitiveLayout<L>),

    /// Common definitions from the Rust standard library
    Std(StdLayout<L>),

    // Custom type definitions
    /// Structs and tuples
    Struct(StructLayout<L>),

    /// Enums
    Enum(EnumLayout<L>),

    /// C-style enumerations (simple named integer constants)
    CEnum(CEnumLayout),

    /// Type currently known only by its name
    Alias { name: String },
}

/// From the Rust standard library:
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub enum PrimitiveLayout<L = ()>
where
    L: Update,
{
    Array(ArrayLayout<L>),
    Bool(()),
    Char(()),
    Float(FloatLayout),
    Function(FunctionLayout<L>),
    Int(IntLayout),
    Never(()),
    Pointer(PointerLayout<L>),
    Reference(ReferenceLayout<L>),
    Slice(SliceLayout<L>),
    /// Technically constructable, `str` is like `[u8; N]`
    /// but where the size is opaque (since utf8 is variable length)
    /// and so rarely seen in the wild. We could have something like `Box<str>`
    /// though.
    Str(()),
    /// A specialization of `Slice` where the referenced type is `str`
    /// Also helps us avoid using the `str` type.
    StrSlice(StrSliceLayout),
    Tuple(TupleLayout<L>),
    Unit(UnitLayout),
    UnsignedInt(UnsignedIntLayout),
    // neverExperimental,
}

impl<L: Update> PrimitiveLayout<L> {
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
pub struct ArrayLayout<L = ()>
where
    L: Update,
{
    pub element_type: Arc<TypeDefinition<L>>,
    pub length: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FloatLayout {
    pub size: usize,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct FunctionLayout<L = ()>
where
    L: Update,
{
    pub return_type: Arc<TypeDefinition<L>>,
    pub arg_types: Vec<Arc<TypeDefinition<L>>>,
}

impl<L: Update> FunctionLayout<L> {
    pub fn display_name(&self) -> String {
        let arg_types = self
            .arg_types
            .iter()
            .map(|arg| arg.display_name())
            .collect::<Vec<_>>()
            .join(", ");

        let mut signature = format!("fn({arg_types})");

        if !matches!(
            self.return_type.layout.as_ref(),
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
pub struct PointerLayout<L = ()>
where
    L: Update,
{
    pub mutable: bool,
    pub pointed_type: Arc<TypeDefinition<L>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ReferenceLayout<L = ()>
where
    L: Update,
{
    /// Is this a mutable reference?
    /// (i.e. `&mut T` vs `&T`)
    pub mutable: bool,

    pub pointed_type: Arc<TypeDefinition<L>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct SliceLayout<L = ()>
where
    L: Update,
{
    pub element_type: Arc<TypeDefinition<L>>,
    pub data_ptr_offset: usize,
    pub length_offset: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StrSliceLayout {
    pub data_ptr_offset: usize,
    pub length_offset: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct TupleLayout<L = ()>
where
    L: Update,
{
    /// List of elements + offsets to their data
    pub elements: Vec<(usize, Arc<TypeDefinition<L>>)>,
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
pub enum StdLayout<L = ()>
where
    L: Update,
{
    SmartPtr(SmartPtrLayout<L>),
    Map(MapLayout<L>),
    Option(OptionLayout<L>),
    Result(ResultLayout<L>),
    String(StringLayout<L>),
    Vec(VecLayout<L>),
}

impl<L: Update> StdLayout<L> {
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
pub struct SmartPtrLayout<L = ()>
where
    L: Update,
{
    pub inner_type: Arc<TypeDefinition<L>>,
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
pub struct MapLayout<L = ()>
where
    L: Update,
{
    pub key_type: Arc<TypeDefinition<L>>,
    pub value_type: Arc<TypeDefinition<L>>,
    pub variant: MapVariant,
}

impl<L: Update> MapLayout<L> {
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
pub struct StringLayout<L = ()>(pub VecLayout<L>)
where
    L: Update;

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct VecLayout<L = ()>
where
    L: Update,
{
    pub length_offset: usize,
    pub data_ptr_offset: usize,
    pub inner_type: Arc<TypeDefinition<L>>,
}

impl<L: Update + Default> VecLayout<L> {
    pub fn new<T: Into<Layout<L>>>(inner_type: T) -> Self {
        Self {
            length_offset: 0,
            data_ptr_offset: 0,
            inner_type: Arc::new(TypeDefinition::new(Default::default(), inner_type.into())),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructLayout<L = ()>
where
    L: Update,
{
    pub name: String,
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<StructField<L>>,
}
#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct StructField<L = ()>
where
    L: Update,
{
    pub name: String,
    pub offset: usize,
    pub ty: Arc<TypeDefinition<L>>,
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
pub struct EnumLayout<L = ()>
where
    L: Update,
{
    pub name: String,
    pub discriminant: Discriminant,
    pub variants: Vec<EnumVariantLayout<L>>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct EnumVariantLayout<L = ()>
where
    L: Update,
{
    pub name: String,
    /// The discriminant value for this variant, if known
    ///
    /// If `None`, we should use the index of the variant
    /// in the `variants` vector as the discriminant value.
    pub discriminant: Option<i128>,
    pub layout: Arc<TypeDefinition<L>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct OptionLayout<L = ()>
where
    L: Update,
{
    pub name: String,
    pub discriminant: Discriminant,
    pub some_offset: usize,
    pub some_type: Arc<TypeDefinition<L>>,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, Update)]
pub struct ResultLayout<L = ()>
where
    L: Update,
{
    pub name: String,
    pub discriminant: Discriminant,
    pub ok_type: Arc<TypeDefinition<L>>,
    pub ok_offset: usize,
    pub err_type: Arc<TypeDefinition<L>>,
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

impl<L: Update + Default> ReferenceLayout<L> {
    pub fn new_mutable<T: Into<Layout<L>>>(pointed_type: T) -> Self {
        Self {
            mutable: true,
            pointed_type: Arc::new(TypeDefinition::new(Default::default(), pointed_type.into())),
        }
    }

    pub fn new_immutable<T: Into<Layout<L>>>(pointed_type: T) -> Self {
        Self {
            mutable: false,
            pointed_type: Arc::new(TypeDefinition::new(Default::default(), pointed_type.into())),
        }
    }
}
