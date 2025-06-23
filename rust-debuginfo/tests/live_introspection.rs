//! Live introspection tests that read debug info from a running process

use anyhow::Result;
use itertools::Itertools;
use rust_debuginfo::{DataResolver, DebugDb, DebugInfo, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// A DataResolver that reads from the current process memory
struct SelfProcessResolver {
    base_address: u64,
}

impl SelfProcessResolver {
    fn new() -> Self {
        // For reading our own process, base address is 0
        Self { base_address: 0 }
    }
}

impl DataResolver for SelfProcessResolver {
    fn base_address(&self) -> u64 {
        self.base_address
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        if size > 4096 {
            return Err(anyhow::anyhow!("Attempting to read too much memory"));
        }
        // Read from our own process memory
        // This is safe because we're only reading memory we own
        let ptr = address as *const u8;
        let mut buffer = vec![0u8; size];

        unsafe {
            std::ptr::copy_nonoverlapping(ptr, buffer.as_mut_ptr(), size);
        }

        tracing::debug!(
            "Read {size} bytes from address {address:#016x}: {:?}",
            buffer
                .iter()
                .chunks(2)
                .into_iter()
                .map(|chunk| {
                    chunk
                        .map(|byte| format!("{:02x}", byte))
                        .collect::<String>()
                })
                .join(" ")
        );

        Ok(buffer)
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        // For testing, we don't need actual register values
        Ok(vec![0; 32])
    }
}

// Test structs that will be included in our test binary
#[derive(Debug)]
struct TestPerson {
    name: String,
    age: u32,
    email: Option<String>,
}

#[derive(Debug)]
struct TestPoint {
    x: f64,
    y: f64,
}

// Struct with only basic types that should work
#[derive(Debug)]
struct TestBasicStruct {
    id: u32,
    count: u64,
    enabled: bool,
    bytes: [u8; 4],
}

#[derive(Debug)]
struct TestComplexData {
    id: u64,
    values: Vec<i32>,
    metadata: HashMap<String, String>,
    location: TestPoint,
}

#[test]
fn test_introspect_string() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    // Get the path to our current test executable
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");

    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data
    let test_string = String::from("Hello, Debugger!");
    let string_ptr = &test_string as *const String as u64;

    // Create resolver for reading our own memory
    let resolver = SelfProcessResolver::new();

    // Find the String type using the public API
    let typedef = debug_info
        .resolve_type("String")?
        .expect("could not resolve String type");
    // Read the string value from memory
    let value = debug_info
        .address_to_value(string_ptr, &typedef, &resolver)
        .expect("Failed to read String from memory");

    // Verify we got the expected value
    match value {
        Value::Scalar { ty, value } if ty == "String" => {
            assert_eq!(value, "\"Hello, Debugger!\"");
        }
        _ => panic!("Expected string value, got: {:?}", value),
    }
    // Keep string alive
    let _ = test_string;
    Ok(())
}

#[test]
fn test_introspect_struct() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data
    let test_person = TestPerson {
        name: String::from("Alice"),
        age: 30,
        email: Some(String::from("alice@example.com")),
    };
    let person_ptr = &test_person as *const TestPerson as u64;

    let resolver = SelfProcessResolver::new();

    // Find TestPerson type using the public API
    let typedef = debug_info
        .resolve_type("TestPerson")?
        .expect("Failed to resolve TestPerson type");

    let value = debug_info
        .address_to_value(person_ptr, &typedef, &resolver)
        .expect("Failed to read TestPerson from memory");

    match value {
        Value::Struct { ty, fields } => {
            assert_eq!(ty, "TestPerson");
            assert!(fields.contains_key("name"));
            assert!(fields.contains_key("age"));
            assert!(fields.contains_key("email"));

            // Check age field
            if let Some(Value::Scalar { ty, value }) = fields.get("age") {
                assert_eq!(ty, "u32");
                assert_eq!(value, "30");
            }
        }
        _ => panic!("Expected struct value, got: {:?}", value),
    }

    // Keep data alive
    let _ = test_person;
    Ok(())
}

#[test]
fn test_introspect_vec() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data
    let test_vec: Vec<i32> = vec![10, 20, 30, 40, 50];
    let vec_ptr = &test_vec as *const Vec<i32> as u64;

    let resolver = SelfProcessResolver::new();

    // Find Vec<i32> type using the public API
    let typedef = debug_info
        .resolve_type("Vec<i32>")?
        .expect("Vec type should be found");

    // Try to read Vec - this will fail when Vec reading isn't implemented
    let value = debug_info.address_to_value(vec_ptr, &typedef, &resolver)?;

    // If we get here, Vec reading is working
    println!("Vec value: {:?}", value);

    // Keep vec alive
    let _ = test_vec;
    Ok(())
}

#[test]
fn test_introspect_option() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Test Option::Some
    let test_some: Option<u32> = Some(42);
    let some_ptr = &test_some as *const Option<u32> as u64;

    // Test Option::None
    let test_none: Option<u32> = None;
    let none_ptr = &test_none as *const Option<u32> as u64;

    let resolver = SelfProcessResolver::new();

    // Find Option<u32> type using the public API
    let typedef = debug_info
        .resolve_type("Option<u32>")?
        .expect("Failed to resolve Option type");

    // Test Some variant - Option is implemented and should work
    let some_value = debug_info
        .address_to_value(some_ptr, &typedef, &resolver)
        .expect("Option::Some reading should be implemented");

    match some_value {
        Value::Scalar { ty, value } => {
            assert_eq!(ty, "u32");
            assert_eq!(value, "42");
            println!("✓ Option::Some correctly read as: {} = {}", ty, value);
        }
        _ => panic!(
            "Expected Some(42) to be read as Scalar {{ ty: u32, value: 42 }}, got: {:?}",
            some_value
        ),
    }

    // Test None variant - Option is implemented and should work
    let none_value = debug_info
        .address_to_value(none_ptr, &typedef, &resolver)
        .expect("Option::None reading should be implemented");

    match none_value {
        Value::Scalar { ty, value } => {
            assert_eq!(ty, "Option");
            assert_eq!(value, "None");
            println!("✓ Option::None correctly read as: {} = {}", ty, value);
        }
        _ => panic!(
            "Expected None to be read as Scalar {{ ty: Option, value: None }}, got: {:?}",
            none_value
        ),
    }

    // Keep data alive
    let _ = (test_some, test_none);
    Ok(())
}

#[test]
fn test_introspect_hashmap() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data
    let mut test_map = HashMap::new();
    test_map.insert("one".to_string(), 1);
    test_map.insert("two".to_string(), 2);
    test_map.insert("three".to_string(), 3);

    let map_ptr = &test_map as *const HashMap<String, i32> as u64;

    let resolver = SelfProcessResolver::new();

    // Find HashMap type using the public API
    let typedef = debug_info
        .resolve_type("HashMap<String, i32>")?
        .expect("HashMap type should be found");

    // Try to read HashMap - this will fail when HashMap reading isn't implemented
    let value = debug_info.address_to_value(map_ptr, &typedef, &resolver)?;

    // If we get here, HashMap reading is working
    println!("HashMap value: {:?}", value);

    // Keep map alive
    let _ = test_map;
    Ok(())
}

#[test]
fn test_introspect_complex_nested_types() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create complex nested data - NOTE: This contains Vec and HashMap which are not implemented yet
    let mut metadata = HashMap::new();
    metadata.insert("version".to_string(), "1.0".to_string());
    metadata.insert("author".to_string(), "test".to_string());

    let test_data = TestComplexData {
        id: 12345,
        values: vec![100, 200, 300],
        metadata,
        location: TestPoint { x: 3.14, y: 2.71 },
    };

    let data_ptr = &test_data as *const TestComplexData as u64;

    let resolver = SelfProcessResolver::new();

    // Find TestComplexData type using the public API
    let typedef = debug_info
        .resolve_type("TestComplexData")?
        .expect("TestComplexData type should be found");

    // Try to read TestComplexData - this will fail when Vec/HashMap reading isn't implemented
    let value = debug_info.address_to_value(data_ptr, &typedef, &resolver)?;

    // If we get here, complex struct reading is working
    println!("TestComplexData value: {:?}", value);

    // Keep data alive
    let _ = test_data;
    Ok(())
}

#[test]
fn test_introspect_smart_pointers() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data with smart pointers
    let test_box: Box<String> = Box::new(String::from("Boxed string"));
    let test_arc: Arc<Vec<i32>> = Arc::new(vec![1, 2, 3]);
    let test_rc: std::rc::Rc<i32> = std::rc::Rc::new(42);
    let test_mutex: std::sync::Mutex<i32> = std::sync::Mutex::new(42);
    let test_cell: std::cell::RefCell<i32> = std::cell::RefCell::new(100);

    let box_ptr = &test_box as *const Box<String> as u64;
    let arc_ptr = &test_arc as *const Arc<Vec<i32>> as u64;
    let rc_ptr = &test_rc as *const std::rc::Rc<i32> as u64;
    let mutex_ptr = &test_mutex as *const std::sync::Mutex<i32> as u64;
    let cell_ptr = &test_cell as *const std::cell::RefCell<i32> as u64;

    let resolver = SelfProcessResolver::new();

    // Test Box - will fail when Box reading isn't implemented
    let box_typedef = debug_info
        .resolve_type("Box<String>")?
        .expect("Box type should be found");

    let box_value = debug_info.address_to_value(box_ptr, &box_typedef, &resolver)?;
    assert_eq!(
        box_value,
        Value::Scalar {
            ty: "Box<String>".to_string(),
            value: "\"Boxed string\"".to_string(),
        }
    );

    // Test Arc - will fail when Arc reading isn't implemented
    let arc_typedef = debug_info
        .resolve_type("Arc<Vec<i32>>")?
        .expect("Arc type should be found");

    let arc_value = debug_info.address_to_value(arc_ptr, &arc_typedef, &resolver)?;
    assert_eq!(
        arc_value,
        Value::Array {
            ty: "Arc<Vec<i32>>".to_string(),
            items: vec![
                Value::Scalar {
                    ty: "i32".to_string(),
                    value: "1".to_string(),
                },
                Value::Scalar {
                    ty: "i32".to_string(),
                    value: "2".to_string(),
                },
                Value::Scalar {
                    ty: "i32".to_string(),
                    value: "3".to_string(),
                }
            ]
        }
    );

    // Test Rc - will fail when Rc reading isn't implemented
    let rc_typedef = debug_info
        .resolve_type("Rc<i32>")?
        .expect("Rc type should be found");

    let rc_value = debug_info.address_to_value(rc_ptr, &rc_typedef, &resolver)?;
    assert_eq!(
        rc_value,
        Value::Scalar {
            ty: "Rc<i32>".to_string(),
            value: "42".to_string(),
        }
    );

    // Test Mutex - will fail when Mutex reading isn't implemented
    let mutex_typedef = debug_info
        .resolve_type("Mutex<i32>")?
        .expect("Mutex type should be found");

    let mutex_value = debug_info.address_to_value(mutex_ptr, &mutex_typedef, &resolver)?;
    assert_eq!(
        mutex_value,
        Value::Scalar {
            ty: "Mutex<i32>".to_string(),
            value: "42".to_string(),
        }
    );

    // Test RefCell - will fail when RefCell reading isn't implemented
    let cell_typedef = debug_info
        .resolve_type("RefCell<i32>")?
        .expect("RefCell type should be found");

    let cell_value = debug_info.address_to_value(cell_ptr, &cell_typedef, &resolver)?;
    assert_eq!(
        cell_value,
        Value::Scalar {
            ty: "RefCell<i32>".to_string(),
            value: "100".to_string(),
        }
    );

    // Keep data alive
    let _ = (test_box, test_arc, test_rc, test_mutex, test_cell);
    Ok(())
}

#[test]
fn test_introspect_basic_struct() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Create test data with only basic types that should be implemented
    let test_basic = TestBasicStruct {
        id: 42,
        count: 12345,
        enabled: true,
        bytes: [0xDE, 0xAD, 0xBE, 0xEF],
    };

    let basic_ptr = &test_basic as *const TestBasicStruct as u64;

    let resolver = SelfProcessResolver::new();

    // Find TestBasicStruct type using the public API
    let typedef = debug_info
        .resolve_type("TestBasicStruct")?
        .expect("Failed to resolve TestBasicStruct type");

    // Try to read the basic struct - will fail if any field types aren't implemented
    let value = debug_info.address_to_value(basic_ptr, &typedef, &resolver)?;

    // If we get here, basic struct reading is working
    println!("TestBasicStruct value: {:?}", value);

    assert_eq!(
        value,
        Value::Struct {
            ty: "TestBasicStruct".to_string(),
            fields: {
                let mut fields = std::collections::BTreeMap::new();
                fields.insert(
                    "id".to_string(),
                    Value::Scalar {
                        ty: "u32".to_string(),
                        value: "42".to_string(),
                    },
                );
                fields.insert(
                    "count".to_string(),
                    Value::Scalar {
                        ty: "u64".to_string(),
                        value: "12345".to_string(),
                    },
                );
                fields.insert(
                    "enabled".to_string(),
                    Value::Scalar {
                        ty: "bool".to_string(),
                        value: "true".to_string(),
                    },
                );
                fields.insert(
                    "bytes".to_string(),
                    Value::Array {
                        ty: "[u8; 4]".to_string(),
                        items: vec!["222", "173", "190", "239"]
                            .into_iter()
                            .map(|v| Value::Scalar {
                                ty: "u8".to_string(),
                                value: v.to_string(),
                            })
                            .collect(),
                    },
                );
                fields
            }
        }
    );

    // Keep data alive
    let _ = test_basic;
    Ok(())
}
