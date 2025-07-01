//! Dynamic tests that introspect the current process
//! These tests must run on each target platform

#[macro_use]
pub mod common;

use common::*;

use anyhow::Result;
use rudy_db::{DataResolver, DebugDb, DebugInfo, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Macro that approximates finding a variable in the current process
/// and reading it from raw memory.
///
/// Uses a little hackery since we don't have a real program counter (PC) in tests.
macro_rules! resolve_variable {
    ($debug_info:ident, $var:ident) => {{
        let resolver = SelfProcessResolver;
        let var_info_pointer = variable_pointer!($debug_info, $var);

        let value = $debug_info
            .read_pointer(&var_info_pointer, &resolver)
            .expect("Failed to read string from memory");

        let value = read_value_recursively(&$debug_info, value, &resolver)
            .expect("Failed to read value recursively");

        value
    }};
}

macro_rules! variable_pointer {
    ($debug_info:ident, $var:ident) => {{
        let resolver = SelfProcessResolver;
        let address = $debug_info
            .resolve_position(file!(), line!() as u64, None)
            .expect("Failed to resolve current position")
            .expect("should resolve current position")
            .address;
        tracing::debug!("Current address: {address:#x}");

        let mut var_info = $debug_info
            .get_variable_at_pc(address, stringify!($var), &resolver)
            .expect("Failed to get variable at address")
            .expect("test_string variable should be found");

        var_info.address = Some(&$var as *const _ as u64);

        var_info
            .as_pointer()
            .expect("Variable should have a pointer")
    }};
}

fn read_value_recursively(
    debug_info: &DebugInfo,
    value: Value,
    resolver: &dyn DataResolver,
) -> Result<Value> {
    match value {
        Value::Pointer(typed_pointer) => {
            let value = debug_info.read_pointer(&typed_pointer, resolver)?;
            read_value_recursively(debug_info, value, resolver)
        }
        v @ Value::Scalar { .. } => Ok(v),
        Value::Array { ty, items } => {
            let items = items
                .into_iter()
                .map(|v| read_value_recursively(debug_info, v, resolver))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array { ty, items })
        }
        Value::Struct { ty, fields } => {
            let fields = fields
                .into_iter()
                .map(|(k, v)| {
                    read_value_recursively(debug_info, v, resolver).map(|v| (k.clone(), v))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok(Value::Struct { ty, fields })
        }
        Value::Tuple { ty, entries } => {
            let entries = entries
                .into_iter()
                .map(|v| read_value_recursively(debug_info, v, resolver))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Tuple { ty, entries })
        }
        Value::Map { ty, entries } => {
            let entries = entries
                .into_iter()
                .map(|(k, v)| {
                    let key = read_value_recursively(debug_info, k, resolver)?;
                    let value = read_value_recursively(debug_info, v, resolver)?;
                    Ok((key, value))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Value::Map { ty, entries })
        }
    }
}

#[macro_export]
macro_rules! setup_db {
    () => {{
        let _guards = setup!();

        let db = Box::new(DebugDb::new());

        let exe_path = std::env::current_exe().expect("Failed to get current exe path");
        let debug_info = DebugInfo::new(Box::leak(db), exe_path.to_str().unwrap())
            .expect("Failed to load debug info");

        (_guards, debug_info)
    }};
}

#[test]
fn test_simple_resolve_debug() -> Result<()> {
    common::init_tracing();

    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    println!("Starting resolve_type call...");
    let start = std::time::Instant::now();

    // Try to resolve a simple type
    match debug_info.resolve_type("u32") {
        Ok(Some((typedef, _))) => {
            println!(
                "Found u32 type in {:?}: {}",
                start.elapsed(),
                typedef.display_name()
            );
        }
        Ok(None) => {
            println!("u32 type not found in {:?}", start.elapsed());
        }
        Err(e) => {
            println!("Error resolving u32 type in {:?}: {}", start.elapsed(), e);
        }
    }

    Ok(())
}

#[test]
fn test_introspect_string() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data
    let test_string = String::from("Hello, Debugger!");

    let value = resolve_variable!(debug_info, test_string);
    insta::assert_debug_snapshot!(value);

    // Keep string alive
    let _ = test_string;
    Ok(())
}

#[test]
fn test_introspect_struct() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data
    let test_person = TestPerson {
        name: String::from("Alice"),
        age: 30,
        email: Some(String::from("alice@example.com")),
    };
    let value = resolve_variable!(debug_info, test_person);

    insta::assert_debug_snapshot!(value);

    // Keep data alive
    let _ = test_person;
    Ok(())
}

#[test]
fn test_introspect_vec() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data
    let test_vec: Vec<i32> = vec![10, 20, 30, 40, 50];
    let value = resolve_variable!(debug_info, test_vec);
    // If we get here, Vec reading is working
    insta::assert_debug_snapshot!(value);

    // Keep vec alive
    let _ = test_vec;
    Ok(())
}

#[test]
fn test_introspect_option() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Test Option::Some
    let test_some: Option<u32> = Some(42);
    let value = resolve_variable!(debug_info, test_some);
    insta::assert_debug_snapshot!(value);

    // Test Option::None
    let test_none: Option<u32> = None;
    let value = resolve_variable!(debug_info, test_none);
    insta::assert_debug_snapshot!(value);

    // Keep data alive
    let _ = (test_some, test_none);
    Ok(())
}

#[test]
fn test_introspect_hashmap() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data
    let mut test_map = HashMap::new();
    test_map.insert("one".to_string(), 1);
    test_map.insert("two".to_string(), 2);
    test_map.insert("three".to_string(), 3);

    let value = resolve_variable!(debug_info, test_map);
    let Value::Map { ty, mut entries } = value else {
        panic!("Expected a Map value, got: {value:?}");
    };
    assert_eq!(ty, "HashMap<String, i32>");
    assert_eq!(entries.len(), 3);
    entries.sort();
    insta::assert_debug_snapshot!(entries);

    // Keep map alive
    let _ = test_map;
    Ok(())
}

#[test]
fn test_introspect_btreemap() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data
    let mut test_map = BTreeMap::new();
    test_map.insert("one".to_string(), 1);
    test_map.insert("two".to_string(), 2);
    test_map.insert("three".to_string(), 3);

    let value = resolve_variable!(debug_info, test_map);
    insta::assert_debug_snapshot!(value);

    // Keep map alive
    let _ = test_map;
    Ok(())
}

#[test]
fn test_introspect_complex_nested_types() {
    let (_guards, debug_info) = setup_db!();

    // Create complex nested data - NOTE: This contains Vec and HashMap which are not implemented yet
    let mut metadata = BTreeMap::new();
    metadata.insert("version".to_string(), "1.0".to_string());
    metadata.insert("author".to_string(), "test".to_string());

    let test_data = TestComplexData {
        id: 12345,
        values: vec![100, 200, 300],
        metadata,
        location: TestPoint {
            x: 3.14001,
            y: 2.71,
        },
    };

    let value = resolve_variable!(debug_info, test_data);
    insta::assert_debug_snapshot!(value);

    // Keep data alive
    let _ = test_data;
}

#[test]
fn test_introspect_smart_pointers() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data with smart pointers
    let test_box: Box<String> = Box::new(String::from("Boxed string"));
    let test_arc: Arc<Vec<i32>> = Arc::new(vec![1, 2, 3]);
    let test_rc: std::rc::Rc<i32> = std::rc::Rc::new(42);
    let test_mutex: std::sync::Mutex<i32> = std::sync::Mutex::new(42);
    let test_cell: std::cell::RefCell<i32> = std::cell::RefCell::new(100);

    let box_value = resolve_variable!(debug_info, test_box);
    insta::assert_debug_snapshot!(box_value);
    let arc_value = resolve_variable!(debug_info, test_arc);
    insta::assert_debug_snapshot!(arc_value);
    let rc_value = resolve_variable!(debug_info, test_rc);
    insta::assert_debug_snapshot!(rc_value);
    let mutex_value = resolve_variable!(debug_info, test_mutex);
    insta::assert_debug_snapshot!(mutex_value);
    let cell_value = resolve_variable!(debug_info, test_cell);
    insta::assert_debug_snapshot!(cell_value);

    // Keep data alive
    let _ = (test_box, test_arc, test_rc, test_mutex, test_cell);
    Ok(())
}

#[test]
fn test_introspect_basic_struct() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Create test data with only basic types that should be implemented
    let test_basic = TestBasicStruct {
        id: 42,
        count: 12345,
        enabled: true,
        bytes: [0xDE, 0xAD, 0xBE, 0xEF],
    };

    let value = resolve_variable!(debug_info, test_basic);
    insta::assert_debug_snapshot!(value);

    // Keep data alive
    let _ = test_basic;
    Ok(())
}

#[test]
fn test_introspect_enums() {
    let (_guards, debug_info) = setup_db!();

    // Test TestEnum variants
    let unit_variant = TestEnum::Unit;
    let tuple_variant = TestEnum::Tuple(42, "test".to_string());
    let struct_variant = TestEnum::Struct {
        x: 3.14002,
        y: 2.71,
    };

    let unit_value = resolve_variable!(debug_info, unit_variant);
    insta::assert_debug_snapshot!(unit_value);

    let tuple_value = resolve_variable!(debug_info, tuple_variant);
    insta::assert_debug_snapshot!(tuple_value);

    let struct_value = resolve_variable!(debug_info, struct_variant);
    insta::assert_debug_snapshot!(struct_value);

    // Test ReprCEnum variants
    let repr_c_unit = ReprCEnum::Unit;
    let repr_c_tuple = ReprCEnum::Tuple(99, "repr_c".to_string());
    let repr_c_struct = ReprCEnum::Struct { x: 1.41, y: 4.13 };

    let repr_c_unit_value = resolve_variable!(debug_info, repr_c_unit);
    insta::assert_debug_snapshot!(repr_c_unit_value);

    let repr_c_tuple_value = resolve_variable!(debug_info, repr_c_tuple);
    insta::assert_debug_snapshot!(repr_c_tuple_value);

    let repr_c_struct_value = resolve_variable!(debug_info, repr_c_struct);
    insta::assert_debug_snapshot!(repr_c_struct_value);

    // Test U8Enum variants
    let u8_first = U8Enum::First;
    let u8_second = U8Enum::Second;
    let u8_third = U8Enum::Third;
    let u8_fifth = U8Enum::Fifth;

    let u8_first_value = resolve_variable!(debug_info, u8_first);
    insta::assert_debug_snapshot!(u8_first_value);

    let u8_second_value = resolve_variable!(debug_info, u8_second);
    insta::assert_debug_snapshot!(u8_second_value);

    let u8_third_value = resolve_variable!(debug_info, u8_third);
    insta::assert_debug_snapshot!(u8_third_value);

    let u8_fifth_value = resolve_variable!(debug_info, u8_fifth);
    insta::assert_debug_snapshot!(u8_fifth_value);

    // Keep all enum values alive
    let _ = (
        unit_variant,
        tuple_variant,
        struct_variant,
        repr_c_unit,
        repr_c_tuple,
        repr_c_struct,
        u8_first,
        u8_second,
        u8_third,
        u8_fifth,
    );
}

// TODO: this seems like it probably works ok on linux but it's _slow_ to parse all
// the symbols in the binary, so we should probably optimize this
#[cfg(target_os = "macos")]
#[test]
fn test_real_method_execution() -> Result<()> {
    let (_guards, debug_info) = setup_db!();

    // Test Vec::len() method execution - this should be simple and safe
    let test_vec = vec![1, 2, 3, 4, 5];

    // Get discovered methods for Vec<i32>

    let vec_pointer = variable_pointer!(debug_info, test_vec);

    let methods = debug_info.discover_methods_for_pointer(&vec_pointer)?;
    println!("Methods found for Vec<i32> ({} total):", methods.len());

    // Show only the first 10 methods to avoid spam
    for (i, method) in methods.iter().take(10).enumerate() {
        println!(
            "  {}: {} - address: {:#x}, callable: {}, signature: {}",
            i + 1,
            method.name,
            method.address,
            method.callable,
            method.signature
        );
    }

    if methods.len() > 10 {
        println!("  ... and {} more methods", methods.len() - 10);
    }

    // Look for specific methods we know should exist
    let len_methods: Vec<_> = methods.iter().filter(|m| m.name.contains("len")).collect();
    println!("\nMethods containing 'len':");
    for method in &len_methods {
        println!(
            "  {} - address: {:#x}, signature: {}",
            method.name, method.address, method.signature
        );
    }

    // Keep test data alive
    let _ = test_vec;
    Ok(())
}

#[test]
fn test_synthetic_methods() -> Result<()> {
    let (_guards, debug_info) = setup_db!();
    let resolver = SelfProcessResolver;

    // Test Vec synthetic methods
    let test_vec = vec![1, 2, 3, 4, 5];

    let test_vec_ptr = variable_pointer!(debug_info, test_vec);
    let vec_ptr = test_vec_ptr.address;
    let vec_type = test_vec_ptr.type_def;

    // Evaluate Vec::len()
    let len_value = rudy_db::evaluate_synthetic_method(vec_ptr, &vec_type, "len", &[], &resolver)?;
    println!("Vec::len() = {len_value:?}");
    assert_eq!(
        len_value,
        Value::Scalar {
            ty: "usize".to_string(),
            value: "5".to_string()
        }
    );

    // Evaluate Vec::capacity()
    let cap_value =
        rudy_db::evaluate_synthetic_method(vec_ptr, &vec_type, "capacity", &[], &resolver)?;
    println!("Vec::capacity() = {cap_value:?}");
    // Capacity should be at least 5
    if let Value::Scalar { value, .. } = &cap_value {
        let cap: usize = value.parse().unwrap();
        assert!(cap >= 5);
    }

    // Evaluate Vec::is_empty()
    let is_empty_value =
        rudy_db::evaluate_synthetic_method(vec_ptr, &vec_type, "is_empty", &[], &resolver)?;
    println!("Vec::is_empty() = {is_empty_value:?}");
    assert_eq!(
        is_empty_value,
        Value::Scalar {
            ty: "bool".to_string(),
            value: "false".to_string()
        }
    );

    // Test String synthetic methods
    let test_string = String::from("Hello, Rust!");
    let string_ptr = variable_pointer!(debug_info, test_string);
    let string_type = string_ptr.type_def;
    let string_ptr = string_ptr.address;

    let string_len =
        rudy_db::evaluate_synthetic_method(string_ptr, &string_type, "len", &[], &resolver)?;
    println!("String::len() = {string_len:?}");
    assert_eq!(
        string_len,
        Value::Scalar {
            ty: "usize".to_string(),
            value: "12".to_string() // "Hello, Rust!" is 12 bytes
        }
    );

    // Test Option synthetic methods
    let some_option: Option<i32> = Some(42);
    let none_option: Option<i32> = None;

    let some_ptr = variable_pointer!(debug_info, some_option);
    let none_ptr = variable_pointer!(debug_info, none_option);
    let option_type = some_ptr.type_def;
    let some_ptr = some_ptr.address;
    let none_ptr = none_ptr.address;

    let is_some =
        rudy_db::evaluate_synthetic_method(some_ptr, &option_type, "is_some", &[], &resolver)?;
    println!("Some(42).is_some() = {is_some:?}");
    assert_eq!(
        is_some,
        Value::Scalar {
            ty: "bool".to_string(),
            value: "true".to_string()
        }
    );

    let is_none =
        rudy_db::evaluate_synthetic_method(none_ptr, &option_type, "is_none", &[], &resolver)?;
    println!("None.is_none() = {is_none:?}");
    assert_eq!(
        is_none,
        Value::Scalar {
            ty: "bool".to_string(),
            value: "true".to_string()
        }
    );

    // TODO: Test Result synthetic methods
    // Skipping Result tests for now since Result layout reading isn't fully implemented
    let ok_result: Result<i32, String> = Ok(42);
    let err_result: Result<i32, String> = Err("error".to_string());

    // Test slice synthetic methods
    let slice: &[i32] = &test_vec[..];
    let slice_ptr = variable_pointer!(debug_info, slice);
    let slice_type = slice_ptr.type_def;
    let slice_ptr = slice_ptr.address;

    let slice_len =
        rudy_db::evaluate_synthetic_method(slice_ptr, &slice_type, "len", &[], &resolver)?;
    println!("&[i32]::len() = {slice_len:?}");
    assert_eq!(
        slice_len,
        Value::Scalar {
            ty: "usize".to_string(),
            value: "5".to_string()
        }
    );

    // TODO: Test array synthetic methods
    // Skipping array tests for now since array types might not be in debug info
    let array: [i32; 3] = [10, 20, 30];

    // Keep all values alive (slice borrows test_vec so we can't move it)
    let _ = (
        &test_vec,
        test_string,
        some_option,
        none_option,
        ok_result,
        err_result,
        slice,
        array,
    );

    Ok(())
}
