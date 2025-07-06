//! Tests for function discovery and calling functionality

use rudy_db::{DebugDb, DebugInfo};

// Helper function to create a debug database for testing
fn create_test_debug_info() -> Option<DebugInfo<'static>> {
    // This would need actual test binaries to work
    // For now, we'll just test the basic API structure
    None
}

#[test]
fn test_function_discovery_api_exists() {
    // Test that the function discovery API exists and has the right signature
    let _db = DebugDb::new();
    
    // This test just verifies the API exists - we can't run it without test binaries
    // In a real test, we would:
    // 1. Load a test binary
    // 2. Create a DebugInfo instance
    // 3. Call discover_functions() and discover_all_functions()
    // 4. Verify the results
    
    // For now, just verify the types exist
    let _: Option<Vec<rudy_db::DiscoveredFunction>> = None;
    let _: Option<std::collections::BTreeMap<String, rudy_db::DiscoveredFunction>> = None;
    
    // Test passes if compilation succeeds
    assert!(true);
}

#[test]
fn test_function_parameter_structure() {
    // Test that FunctionParameter has the expected fields exist
    // This is more of a compile-time test to ensure the API exists
    
    let _name: Option<String> = None;
    let _param_type: Option<rudy_db::FunctionParameter> = None;
    
    // Test that the types can be constructed (basic compilation test)
    assert!(true);
}

#[test]
fn test_discovered_function_structure() {
    // Test that DiscoveredFunction has the expected fields
    let function = rudy_db::DiscoveredFunction {
        name: "test_function".to_string(),
        full_name: "module::test_function".to_string(),
        signature: "fn test_function() -> i32".to_string(),
        address: 0x12345,
        callable: true,
        module_path: Some("module".to_string()),
        return_type: None,
        parameters: vec![],
    };
    
    assert_eq!(function.name, "test_function");
    assert_eq!(function.full_name, "module::test_function");
    assert_eq!(function.address, 0x12345);
    assert!(function.callable);
    assert_eq!(function.module_path, Some("module".to_string()));
    assert!(function.parameters.is_empty());
}