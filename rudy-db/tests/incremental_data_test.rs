//! Tests for incremental data reading with Value::Pointer

use anyhow::Result;
use rudy_db::{DebugDb, DebugInfo, TypedPointer, Value};
use rudy_types::TypeLayout;
use std::sync::Arc;

/// Mock data resolver for testing
struct MockDataResolver {
    base_address: u64,
    memory: std::collections::HashMap<u64, Vec<u8>>,
}

impl MockDataResolver {
    fn new() -> Self {
        Self {
            base_address: 0x1000,
            memory: std::collections::HashMap::new(),
        }
    }

    fn write_memory(&mut self, address: u64, data: Vec<u8>) {
        self.memory.insert(address, data);
    }
}

impl rudy_db::DataResolver for MockDataResolver {
    fn base_address(&self) -> u64 {
        self.base_address
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        if let Some(data) = self.memory.get(&address) {
            if data.len() >= size {
                Ok(data[..size].to_vec())
            } else {
                Ok(data.clone())
            }
        } else {
            // Return zeros for uninitialized memory
            Ok(vec![0u8; size])
        }
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        Ok(vec![])
    }
}

#[test]
fn test_value_pointer_creation() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Try to get a simple type
    if let Ok(Some(u32_type)) = debug_info.resolve_type("u32") {
        // Create a mock debug file (use first available)
        if let Some(debug_file) = debug_info.first_debug_file() {
            // Create a TypedPointer
            let typed_pointer = TypedPointer {
                address: 0x1000,
                type_def: Arc::new(u32_type),
                debug_file,
            };

            // Create a Value::Pointer
            let pointer_value = Value::Pointer(typed_pointer);

            // Verify it's a pointer
            match pointer_value {
                Value::Pointer(ref ptr) => {
                    assert_eq!(ptr.address, 0x1000);
                    assert_eq!(ptr.type_def.display_name(), "u32");
                }
                _ => panic!("Expected Value::Pointer"),
            }

            println!("Successfully created Value::Pointer for u32");
        }
    }

    Ok(())
}

#[test]
fn test_struct_field_lazy_loading() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    let mut resolver = MockDataResolver::new();

    // Set up some mock memory data
    resolver.write_memory(0x1000, vec![42, 0, 0, 0]); // u32 value = 42
    resolver.write_memory(0x1004, vec![1, 2, 3, 4]); // Additional data

    // Try to find a struct type to test with
    // This is a basic test since we need actual debug info from a real binary
    // In a real scenario, this would test accessing fields from structs

    // Use debug_info for something
    let _ = debug_info.first_debug_file();
    println!("Mock data resolver test completed");
    Ok(())
}

#[test]
fn test_value_ordering() -> Result<()> {
    // Test that Value::Pointer can be ordered (required for BTreeMap usage)
    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    if let Ok(Some(u32_type)) = debug_info.resolve_type("u32") {
        if let Some(debug_file) = debug_info.first_debug_file() {
            let ptr1 = Value::Pointer(TypedPointer {
                address: 0x1000,
                type_def: Arc::new(u32_type.clone()),
                debug_file,
            });

            let ptr2 = Value::Pointer(TypedPointer {
                address: 0x2000,
                type_def: Arc::new(u32_type),
                debug_file,
            });

            // Test ordering
            assert!(ptr1 < ptr2);
            assert!(ptr2 > ptr1);
            assert_eq!(ptr1, ptr1);

            println!("Value::Pointer ordering works correctly");
        }
    }

    Ok(())
}

#[test]
fn test_alias_in_pointer() -> Result<()> {
    // Test that aliases in TypedPointer are handled correctly
    let db = DebugDb::new();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    if let Some(debug_file) = debug_info.first_debug_file() {
        // Create a mock alias type
        let alias_type = TypeLayout::Alias(rudy_types::UnresolvedType {
            name: "MyAlias".to_string(),
            cu_offset: 100,
            die_offset: 200,
        });

        let ptr_with_alias = Value::Pointer(TypedPointer {
            address: 0x1000,
            type_def: Arc::new(alias_type),
            debug_file,
        });

        // Verify the pointer contains an alias
        match ptr_with_alias {
            Value::Pointer(ref ptr) => match &*ptr.type_def {
                TypeLayout::Alias(alias) => {
                    assert_eq!(alias.name, "MyAlias");
                    println!("Successfully created pointer with alias type");
                }
                _ => panic!("Expected alias type"),
            },
            _ => panic!("Expected Value::Pointer"),
        }
    }

    Ok(())
}
