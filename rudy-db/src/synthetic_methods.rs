//! Synthetic methods that can be evaluated without execution
//!
//! These are methods we can compute directly from debug information
//! without needing to execute code in the target process.
//!
//! ## Current Implementation
//!
//! We currently support synthetic methods for:
//! - **Vec<T>**: `len()`, `capacity()`, `is_empty()`
//! - **String**: `len()`, `is_empty()`
//! - **Option<T>**: `is_some()`, `is_none()`
//! - **Result<T,E>**: `is_ok()`, `is_err()` (partial - needs Result layout work)
//! - **&[T]**: `len()`, `is_empty()`
//! - **&str**: `len()`, `is_empty()`
//! - **[T; N]**: `len()` (when array types are available in debug info)
//!
//! ## Future Methods to Implement
//!
//! ### Standard Collections
//! - **HashMap<K,V>** / **BTreeMap<K,V>**: `len()`, `is_empty()`, `contains_key()`
//! - **HashSet<T>** / **BTreeSet<T>**: `len()`, `is_empty()`, `contains()`
//! - **VecDeque<T>**: `len()`, `capacity()`, `is_empty()`
//! - **LinkedList<T>**: `len()`, `is_empty()`
//!
//! ### Smart Pointers
//! - **Box<T>**: `is_null()` (check for null pointer)
//! - **Rc<T>** / **Arc<T>**: `strong_count()`, `weak_count()`
//! - **RefCell<T>**: `try_borrow()` (check if currently borrowed)
//! - **Mutex<T>** / **RwLock<T>**: `is_poisoned()`
//!
//! ### Iterators
//! - **Range<T>**: `len()`, `is_empty()`, `contains()`
//! - **Chars**: `count()` (for string char iterators)
//!
//! ### Networking & I/O
//! - **SocketAddr**: `ip()`, `port()`, `is_ipv4()`, `is_ipv6()`
//! - **IpAddr**: `is_loopback()`, `is_multicast()`, `is_global()`
//! - **Path** / **PathBuf**: `is_absolute()`, `is_relative()`, `exists()` (if we can stat)
//!
//! ### Time & Duration
//! - **Duration**: `as_secs()`, `as_millis()`, `as_micros()`, `as_nanos()`, `is_zero()`
//! - **Instant**: `elapsed()` (if we can get current time)
//! - **SystemTime**: `duration_since()` (relative to UNIX_EPOCH)
//!
//! ### Concurrency Primitives
//! - **AtomicBool** / **AtomicI32** etc: `load()` (read current value)
//! - **Barrier**: `is_leader()` (for barrier synchronization)
//!
//! ### Custom Methods with Arguments
//! - **Vec<T>**: `get(index)` - bounds-checked element access
//! - **String**: `char_at(index)` - get character at byte index
//! - **HashMap<K,V>**: `get(key)` - look up value by key
//! - **Range<T>**: `contains(value)` - check if value is in range
//!
//! ### Complex Computed Properties
//! - **String**: `char_len()` - length in Unicode characters (vs bytes)
//! - **Vec<T>**: `remaining_capacity()` - capacity minus length
//! - **Path**: `file_name()`, `extension()`, `parent()` (string operations on path)
//!
//! ## Implementation Notes
//!
//! Synthetic methods should only be implemented when:
//! 1. The computation can be done entirely from memory layout and debug info
//! 2. The result is deterministic and side-effect free
//! 3. The performance is reasonable (no complex algorithms)
//! 4. The method provides significant debugging value
//!
//! Methods requiring system calls, file I/O, network access, or complex
//! computations should use actual method execution instead.

use anyhow::{Result, anyhow};
use rudy_dwarf::{Die, types::DieTypeDefinition};
use rudy_types::{Layout, Location, StdLayout};

use crate::{DataResolver, Value};

/// A synthetic method that can be evaluated without execution
#[derive(Debug, Clone)]
pub struct SyntheticMethod {
    /// The method name (e.g., "len", "is_empty")
    pub name: &'static str,

    /// Human-readable signature for display
    pub signature: &'static str,

    /// Whether this method requires arguments
    pub takes_args: bool,
}

/// Get all synthetic methods available for a given type
pub fn get_synthetic_methods<L: Location>(type_layout: &Layout<L>) -> Vec<SyntheticMethod> {
    match type_layout {
        Layout::Std(std_layout) => match std_layout {
            StdLayout::Vec(_) => vec![
                SyntheticMethod {
                    name: "len",
                    signature: "fn len(&self) -> usize",
                    takes_args: false,
                },
                SyntheticMethod {
                    name: "capacity",
                    signature: "fn capacity(&self) -> usize",
                    takes_args: false,
                },
                SyntheticMethod {
                    name: "is_empty",
                    signature: "fn is_empty(&self) -> bool",
                    takes_args: false,
                },
            ],
            StdLayout::String(_) => vec![
                SyntheticMethod {
                    name: "len",
                    signature: "fn len(&self) -> usize",
                    takes_args: false,
                },
                SyntheticMethod {
                    name: "is_empty",
                    signature: "fn is_empty(&self) -> bool",
                    takes_args: false,
                },
            ],
            StdLayout::Option(_) => vec![
                SyntheticMethod {
                    name: "is_some",
                    signature: "fn is_some(&self) -> bool",
                    takes_args: false,
                },
                SyntheticMethod {
                    name: "is_none",
                    signature: "fn is_none(&self) -> bool",
                    takes_args: false,
                },
            ],
            StdLayout::Result(_) => vec![
                SyntheticMethod {
                    name: "is_ok",
                    signature: "fn is_ok(&self) -> bool",
                    takes_args: false,
                },
                SyntheticMethod {
                    name: "is_err",
                    signature: "fn is_err(&self) -> bool",
                    takes_args: false,
                },
            ],
            StdLayout::Map(_) => vec![
                // HashMap/BTreeMap methods require understanding internal structure
                // which is complex and varies by implementation
            ],
            _ => vec![],
        },
        Layout::Primitive(prim_layout) => {
            use rudy_types::PrimitiveLayout;
            match prim_layout {
                PrimitiveLayout::Slice(_) | PrimitiveLayout::StrSlice(_) => vec![
                    SyntheticMethod {
                        name: "len",
                        signature: "fn len(&self) -> usize",
                        takes_args: false,
                    },
                    SyntheticMethod {
                        name: "is_empty",
                        signature: "fn is_empty(&self) -> bool",
                        takes_args: false,
                    },
                ],
                PrimitiveLayout::Array(_) => vec![SyntheticMethod {
                    name: "len",
                    signature: "fn len(&self) -> usize",
                    takes_args: false,
                }],
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// Evaluate a synthetic method call
pub fn evaluate_synthetic_method(
    address: u64,
    def: &DieTypeDefinition,
    method: &str,
    _args: &[Value], // For future use when we support methods with arguments
    resolver: &dyn DataResolver,
) -> Result<Value> {
    match def.layout.as_ref() {
        Layout::Std(std_layout) => match std_layout {
            StdLayout::Vec(vec_layout) => {
                evaluate_vec_method(address, vec_layout, method, resolver)
            }
            StdLayout::String(string_layout) => {
                evaluate_string_method(address, string_layout, method, resolver)
            }
            StdLayout::Option(option_layout) => {
                evaluate_option_method(address, option_layout, method, resolver)
            }
            StdLayout::Result(result_layout) => {
                evaluate_result_method(address, result_layout, method, resolver)
            }
            StdLayout::Map(map_layout) => {
                evaluate_map_method(address, map_layout, method, resolver)
            }
            _ => Err(anyhow!(
                "No synthetic method '{}' for type {}",
                method,
                def.display_name()
            )),
        },
        Layout::Primitive(prim_layout) => {
            use rudy_types::PrimitiveLayout;
            match prim_layout {
                PrimitiveLayout::Slice(slice_layout) => {
                    evaluate_slice_method(address, slice_layout, method, resolver)
                }
                PrimitiveLayout::StrSlice(_) => {
                    evaluate_str_slice_method(address, method, resolver)
                }
                PrimitiveLayout::Array(array_layout) => evaluate_array_method(array_layout, method),
                _ => Err(anyhow!(
                    "No synthetic method '{}' for type {}",
                    method,
                    def.display_name()
                )),
            }
        }
        _ => Err(anyhow!(
            "No synthetic methods for type {}",
            def.display_name()
        )),
    }
}

fn evaluate_vec_method(
    address: u64,
    vec_layout: &rudy_types::VecLayout<Die>,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    match method {
        "len" => {
            let len_address = address + vec_layout.length_offset as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "usize".to_string(),
                value: len.to_string(),
            })
        }
        "capacity" => {
            // Vec layout typically has: data_ptr, length, capacity
            // capacity is usually after length
            let cap_offset = vec_layout.capacity_offset;
            let cap_address = address + cap_offset as u64;
            let cap_bytes = resolver.read_memory(cap_address, std::mem::size_of::<usize>())?;
            let cap = usize_from_bytes(&cap_bytes)?;
            Ok(Value::Scalar {
                ty: "usize".to_string(),
                value: cap.to_string(),
            })
        }
        "is_empty" => {
            let len_address = address + vec_layout.length_offset as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "bool".to_string(),
                value: (len == 0).to_string(),
            })
        }
        _ => Err(anyhow!("Unknown synthetic method '{}' for Vec", method)),
    }
}

fn evaluate_string_method(
    address: u64,
    string_layout: &rudy_types::StringLayout<Die>,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    // String is just a Vec<u8> internally
    let vec_layout = &string_layout.0;
    match method {
        "len" => evaluate_vec_method(address, vec_layout, "len", resolver),
        "is_empty" => evaluate_vec_method(address, vec_layout, "is_empty", resolver),
        "capacity" => evaluate_vec_method(address, vec_layout, "capacity", resolver),
        _ => Err(anyhow!("Unknown synthetic method '{}' for String", method)),
    }
}

fn evaluate_option_method(
    address: u64,
    option_layout: &rudy_types::OptionLayout<Die>,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    // Read the discriminant to check Some vs None
    let discriminant_bytes = resolver.read_memory(
        address + option_layout.discriminant.offset as u64,
        option_layout.discriminant.size(),
    )?;

    let discriminant_value = match discriminant_bytes.len() {
        1 => discriminant_bytes[0] as u64,
        2 => u16::from_le_bytes(discriminant_bytes.try_into().unwrap()) as u64,
        4 => u32::from_le_bytes(discriminant_bytes.try_into().unwrap()) as u64,
        8 => u64::from_le_bytes(discriminant_bytes.try_into().unwrap()),
        _ => return Err(anyhow!("Unexpected discriminant size")),
    };

    // For Option, typically discriminant 0 = None, 1 = Some
    // This is a common pattern but may vary based on compiler optimization
    match method {
        "is_some" => Ok(Value::Scalar {
            ty: "bool".to_string(),
            value: (discriminant_value != 0).to_string(),
        }),
        "is_none" => Ok(Value::Scalar {
            ty: "bool".to_string(),
            value: (discriminant_value == 0).to_string(),
        }),
        _ => Err(anyhow!("Unknown synthetic method '{}' for Option", method)),
    }
}

fn evaluate_result_method(
    address: u64,
    result_layout: &rudy_types::ResultLayout<Die>,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    // Read the discriminant to check Ok vs Err
    let discriminant_bytes = resolver.read_memory(
        address + result_layout.discriminant.offset as u64,
        result_layout.discriminant.size(),
    )?;

    let discriminant_value = match discriminant_bytes.len() {
        1 => discriminant_bytes[0] as u64,
        2 => u16::from_le_bytes(discriminant_bytes.clone().try_into().unwrap()) as u64,
        4 => u32::from_le_bytes(discriminant_bytes.clone().try_into().unwrap()) as u64,
        8 => u64::from_le_bytes(discriminant_bytes.clone().try_into().unwrap()),
        _ => return Err(anyhow!("Unexpected discriminant size")),
    };

    // Debug: print the discriminant value and bytes
    tracing::debug!(
        "Result discriminant bytes: {:?}, value: {:#x}, at offset {}",
        discriminant_bytes,
        discriminant_value,
        result_layout.discriminant.offset
    );

    // Result uses a special encoding where the discriminant is part of the payload
    // For Result<T, E>, when it's Ok, the discriminant area contains the Ok value
    // When it's Err, it uses a special marker (often 0x8000000000000000 for 64-bit)
    // This is an optimization to avoid wasting space
    match method {
        "is_ok" => {
            // Check if the high bit is NOT set (indicating Ok variant)
            let is_ok = match discriminant_bytes.len() {
                8 => (discriminant_value & 0x8000000000000000) == 0,
                4 => (discriminant_value & 0x80000000) == 0,
                _ => discriminant_value == 0,
            };
            Ok(Value::Scalar {
                ty: "bool".to_string(),
                value: is_ok.to_string(),
            })
        }
        "is_err" => {
            // Check if the high bit IS set (indicating Err variant)
            let is_err = match discriminant_bytes.len() {
                8 => (discriminant_value & 0x8000000000000000) != 0,
                4 => (discriminant_value & 0x80000000) != 0,
                _ => discriminant_value != 0,
            };
            Ok(Value::Scalar {
                ty: "bool".to_string(),
                value: is_err.to_string(),
            })
        }
        _ => Err(anyhow!("Unknown synthetic method '{}' for Result", method)),
    }
}

fn evaluate_map_method(
    _address: u64,
    _map_layout: &rudy_types::MapLayout<Die>,
    method: &str,
    _resolver: &dyn DataResolver,
) -> Result<Value> {
    // HashMap/BTreeMap methods are more complex to implement
    // as they require understanding the internal structure
    match method {
        "len" | "is_empty" => Err(anyhow!(
            "HashMap/BTreeMap synthetic methods not yet implemented"
        )),
        _ => Err(anyhow!("Unknown synthetic method '{}' for Map", method)),
    }
}

fn evaluate_slice_method(
    address: u64,
    _slice_layout: &rudy_types::SliceLayout<Die>,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    match method {
        "len" => {
            // Slice is a fat pointer: (data_ptr, length)
            // Length is at offset 8 (after the pointer)
            let len_address = address + std::mem::size_of::<usize>() as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "usize".to_string(),
                value: len.to_string(),
            })
        }
        "is_empty" => {
            let len_address = address + std::mem::size_of::<usize>() as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "bool".to_string(),
                value: (len == 0).to_string(),
            })
        }
        _ => Err(anyhow!("Unknown synthetic method '{}' for slice", method)),
    }
}

fn evaluate_str_slice_method(
    address: u64,
    method: &str,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    // &str is a fat pointer just like slices: (data_ptr, length)
    match method {
        "len" => {
            // Length is at offset 8 (after the pointer)
            let len_address = address + std::mem::size_of::<usize>() as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "usize".to_string(),
                value: len.to_string(),
            })
        }
        "is_empty" => {
            let len_address = address + std::mem::size_of::<usize>() as u64;
            let len_bytes = resolver.read_memory(len_address, std::mem::size_of::<usize>())?;
            let len = usize_from_bytes(&len_bytes)?;
            Ok(Value::Scalar {
                ty: "bool".to_string(),
                value: (len == 0).to_string(),
            })
        }
        _ => Err(anyhow!("Unknown synthetic method '{}' for &str", method)),
    }
}

fn evaluate_array_method(
    array_layout: &rudy_types::ArrayLayout<Die>,
    method: &str,
) -> Result<Value> {
    match method {
        "len" => Ok(Value::Scalar {
            ty: "usize".to_string(),
            value: array_layout.length.to_string(),
        }),
        _ => Err(anyhow!("Unknown synthetic method '{}' for array", method)),
    }
}

fn usize_from_bytes(bytes: &[u8]) -> Result<usize> {
    if bytes.len() == 8 {
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()) as usize)
    } else if bytes.len() == 4 {
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
    } else {
        Err(anyhow!("Unexpected size for usize: {} bytes", bytes.len()))
    }
}
