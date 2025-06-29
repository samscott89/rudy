//! Dynamic tests that introspect the current process
//! These tests must run on each target platform

use rudy_db::*;

#[macro_use]
pub mod common;

#[test]
fn test_enum_type_resolution() {
    let _guards = setup!();

    // test enum introspection for TestEnum, ReprCEnum, and U8Enum
    // we'll introspect the current process to find these types

    let db = DebugDb::new();

    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let debug_info =
        DebugInfo::new(&db, exe_path.to_str().unwrap()).expect("Failed to load debug info");

    // Test TestEnum variants
    let _unit = common::TestEnum::Unit;

    // Find TestEnum type
    let test_enum_typedef = debug_info
        .resolve_type("TestEnum")
        .expect("Failed to resolve TestEnum")
        .expect("TestEnum type should be found");

    insta::assert_debug_snapshot!(test_enum_typedef);

    // Test ReprCEnum variants
    let _repr_c_unit = common::ReprCEnum::Unit;

    let repr_c_typedef = debug_info
        .resolve_type("ReprCEnum")
        .expect("Failed to resolve ReprCEnum")
        .expect("ReprCEnum type should be found");

    insta::assert_debug_snapshot!(repr_c_typedef);

    // we'll also test our special-cased enums Option and Result

    let _option: Option<i32> = Some(42);
    let option_typedef = debug_info
        .resolve_type("Option<i32>")
        .expect("Failed to resolve Option<i32>")
        .expect("Option<i32> type should be found");

    insta::assert_debug_snapshot!(option_typedef);

    let _result: Result<i32, String> = Ok(42);
    let result_typedef = debug_info
        .resolve_type("Result<i32, String>")
        .expect("Failed to resolve Result<i32, String>")
        .expect("Result<i32, String> type should be found");

    insta::assert_debug_snapshot!(result_typedef);

    // Test U8Enum variants
    let _u8_first = common::U8Enum::First;

    let u8_enum_typedef = debug_info
        .resolve_type("U8Enum")
        .expect("Failed to resolve U8Enum")
        .expect("U8Enum type should be found");

    insta::assert_debug_snapshot!(u8_enum_typedef);
}

#[test]
fn test_method_discovery_current_platform() {
    let _guards = setup!();

    // This test uses a specific example binary for the current platform
    let current_target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => panic!("Unsupported platform"),
    };

    let artifacts = common::artifacts_dir();
    let path = artifacts.join(current_target).join("lldb_demo");

    if !path.exists() {
        eprintln!(
            "Warning: lldb_demo binary not found at: {}\n\
             Skipping test. Run `cargo xtask build-test-artifacts --current-platform` to generate it.",
            path.display()
        );
        return;
    }

    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, path.to_str().unwrap()).unwrap();

    // Test 1: Debug version - capture all symbol analysis results
    let symbol_analysis_results = debug_info
        .discover_all_methods_debug()
        .expect("Symbol analysis should succeed");

    salsa::attach(&db, || {
        insta::assert_debug_snapshot!(symbol_analysis_results)
    });

    // Test 2: Discover all methods in the binary (original test)
    let methods_by_type = debug_info
        .discover_all_methods()
        .expect("Method discovery should succeed");

    insta::assert_debug_snapshot!(methods_by_type);

    // Test 3: Test specific type resolution and method discovery
    let (session_type, _) = debug_info
        .resolve_type("Session")
        .expect("Type resolution should succeed")
        .expect("Session type should be found");

    let methods = debug_info
        .discover_methods_for_type(&session_type)
        .expect("Method discovery for Session should succeed");

    insta::assert_debug_snapshot!(methods);
}
