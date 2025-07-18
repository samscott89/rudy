//! Static tests that analyze pre-built binaries
//! These tests can run on any platform as they only read debug info

use rstest::rstest;
use rstest_reuse::{self, *};
use rudy_db::*;

#[macro_use]
pub mod common;

use common::root_artifacts_dir;

pub fn binary_path(target: &str, example: &str) -> String {
    let artifacts = root_artifacts_dir();
    let binary_path = artifacts.join(target).join(example);

    if !binary_path.exists() {
        panic!(
            "Test binary not found at: {}\n\
             Please run `cargo xtask build-test-artifacts` to generate test binaries.",
            binary_path.display()
        );
    }

    binary_path.to_str().unwrap().to_string()
}

macro_rules! salsa_debug_snapshot {
    ($db:ident, $expr:expr) => {
        // snapshot while in the context of a db
        // so we get nice debug output
        salsa::attach(&$db, || {
            insta::assert_debug_snapshot!($expr);
        })
    };
}

#[template]
#[rstest]
#[case("aarch64-unknown-linux-gnu")]
#[case("x86_64-unknown-linux-gnu")]
// we can only really run these on macOS when we have the sources
// installed, since macos relies on debug symbols living
// alongside the standard libraries
// on mac we can run all of these, since the linux debug info is
// self-contained
#[cfg_attr(target_os = "macos", case("aarch64-apple-darwin"))]
#[cfg_attr(target_os = "macos", case("x86_64-apple-darwin"))]
pub fn binary_target(#[case] target: &'static str) {}

#[apply(binary_target)]
fn test_resolve_function(#[case] target: &'static str) {
    let _guards = setup!(target);
    let path = binary_path(target, "simple_test");
    let db = common::debug_db(Some(target));
    let resolver = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(resolver.find_function_by_name("main").unwrap());
    insta::assert_debug_snapshot!(resolver.find_function_by_name("function_call").unwrap());
}

#[apply(binary_target)]
fn test_resolve_position(#[case] target: &'static str) {
    let _guards = setup!(target);
    let path = binary_path(target, "simple_test");
    let db = common::debug_db(Some(target));
    let resolver = DebugInfo::new(&db, &path).unwrap();

    let platform_file = "simple_test.rs";
    let expected = common::workspace_dir()
        .join("crates/rudy-test-examples/examples/simple_test.rs")
        .to_str()
        .expect("Failed to convert path to string")
        .to_string();

    // should be the position of the `let y = x + 1;` line
    let addrs = resolver
        .find_address_from_source_location(platform_file, 2, None)
        .unwrap()
        .unwrap();
    salsa_debug_snapshot!(db, addrs);
    assert_eq!(
        resolver
            .address_to_location(addrs.address)
            .unwrap()
            .unwrap(),
        ResolvedLocation {
            function: "simple_test::function_call".to_string(),
            file: expected.clone(),
            line: 2,
        }
    );

    // should be the position of the `const Z: u64 = 0xdeadbeef;` line
    let addrs = resolver
        .find_address_from_source_location(platform_file, 11, None)
        .unwrap()
        .unwrap();
    salsa_debug_snapshot!(db, addrs);

    assert_eq!(
        resolver
            .address_to_location(addrs.address)
            .unwrap()
            .unwrap(),
        ResolvedLocation {
            function: "simple_test::main".to_string(),
            file: expected.clone(),
            line: 12,
        }
    );
}

#[apply(binary_target)]
fn test_load_file(#[case] target: &'static str) {
    let _guards = setup!(target);

    let path = binary_path(target, "simple_test");

    let db = common::debug_db(Some(target));
    let parsed = DebugInfo::new(&db, &path).unwrap();

    // let index = parsed.

    salsa_debug_snapshot!(db, parsed);
}

#[apply(binary_target)]
fn test_enum_type_resolution(#[case] target: &'static str) {
    let _guards = setup!(target);

    let db = common::debug_db(Some(target));
    let exe_path = binary_path(target, "enums");
    let debug_info = DebugInfo::new(&db, &exe_path).expect("Failed to load debug info");

    // Find TestEnum type
    let test_enum_typedef = debug_info
        .lookup_type_by_name("enums::TestEnum")
        .expect("Failed to resolve enums::TestEnum")
        .expect("enums::TestEnum type should be found");

    salsa_debug_snapshot!(db, test_enum_typedef);

    let repr_c_typedef = debug_info
        .lookup_type_by_name("enums::ReprCEnum")
        .expect("Failed to resolve enums::ReprCEnum")
        .expect("enums::ReprCEnum type should be found");

    salsa_debug_snapshot!(db, repr_c_typedef);

    // we'll also test our special-cased enums Option and Result
    let option_typedef = debug_info
        .lookup_type_by_name("core::option::Option<i32>")
        .expect("Failed to resolve core::option::Option<i32>")
        .expect("core::option::Option<i32> type should be found");

    salsa_debug_snapshot!(db, option_typedef);

    let result_typedef = debug_info
        .lookup_type_by_name("core::result::Result<i32, alloc::string::String>")
        .expect("Failed to resolve core::result::Result<i32, alloc::string::String>")
        .expect("core::result::Result<i32, alloc::string::String> type should be found");

    salsa_debug_snapshot!(db, result_typedef);

    // Test U8Enum variants
    let u8_enum_typedef = debug_info
        .lookup_type_by_name("enums::U8Enum")
        .expect("Failed to resolve enums::U8Enum")
        .expect("enums::U8Enum type should be found");

    salsa_debug_snapshot!(db, u8_enum_typedef);
}

#[apply(binary_target)]
fn test_method_discovery(#[case] target: &'static str) {
    let _guards = setup!(target);

    let db = common::debug_db(Some(target));
    let exe_path = binary_path(target, "method_discovery");
    let debug_info = DebugInfo::new(&db, &exe_path).expect("Failed to load debug info");

    // Find `test_all_methods` function
    let test_all_methods = debug_info
        .find_function_by_name("method_discovery::test_all_methods")
        .expect("Failed to resolve method_discovery::test_all_methods")
        .expect("method_discovery::test_all_methods function should be found");

    let session_param = test_all_methods
        .params
        .iter()
        .find(|p| p.name == "session")
        .expect("Session parameter not found");
    let methods = debug_info
        .discover_methods_for_type(&session_param.ty)
        .expect("Failed to discover methods for session type");

    salsa_debug_snapshot!(db, methods);
}

#[apply(binary_target)]
fn test_function_discovery(#[case] target: &'static str) {
    let _guards = setup!(target);

    let db = common::debug_db(Some(target));
    let exe_path = binary_path(target, "simple_test");
    let debug_info = DebugInfo::new(&db, &exe_path).expect("Failed to load debug info");

    // Test discovering functions by pattern
    let main_functions = debug_info
        .discover_functions("main")
        .expect("Failed to discover main functions");

    // Should find at least the main function
    assert!(
        !main_functions.is_empty(),
        "Should find at least one main function"
    );

    let main_function = &main_functions[0];
    assert_eq!(main_function.name, "main");
    assert!(main_function.callable);
    assert!(main_function.address > 0);

    salsa_debug_snapshot!(db, main_functions);

    // Test discovering functions by partial name
    let function_call_functions = debug_info
        .discover_functions("function_call")
        .expect("Failed to discover function_call functions");

    assert!(
        !function_call_functions.is_empty(),
        "Should find function_call function"
    );

    let function_call = &function_call_functions[0];
    assert_eq!(function_call.name, "function_call");
    assert!(function_call.callable);
    assert!(function_call.address > 0);

    salsa_debug_snapshot!(db, function_call_functions);

    // Test discovering all functions
    let all_functions = debug_info
        .discover_all_functions()
        .expect("Failed to discover all functions");

    // Should have many functions
    assert!(all_functions.len() > 2, "Should find multiple functions");

    // Main function should be in the results
    assert!(
        all_functions.contains_key("simple_test::main"),
        "Should contain main function"
    );

    // Snapshot a subset for testing
    let subset: std::collections::BTreeMap<_, _> = all_functions
        .iter()
        .filter(|(name, _)| name.contains("simple_test"))
        .take(10)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    salsa_debug_snapshot!(db, subset);
}
