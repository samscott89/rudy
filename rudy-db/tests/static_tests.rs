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

    insta::assert_debug_snapshot!(resolver.resolve_function("main").unwrap());
    insta::assert_debug_snapshot!(resolver.resolve_function("function_call").unwrap());
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
        .resolve_position(platform_file, 2, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);
    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "simple_test::function_call".to_string(),
            file: expected.clone(),
            line: 2,
        }
    );

    // should be the position of the `const Z: u64 = 0xdeadbeef;` line
    let addrs = resolver
        .resolve_position(platform_file, 11, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);

    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
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

    insta::assert_debug_snapshot!(parsed);
}

#[apply(binary_target)]
fn test_enum_type_resolution(#[case] target: &'static str) {
    let _guards = setup!(target);

    let db = common::debug_db(Some(target));
    let exe_path = binary_path(target, "enums");
    let debug_info = DebugInfo::new(&db, &exe_path).expect("Failed to load debug info");

    // Find TestEnum type
    let (test_enum_typedef, _) = debug_info
        .resolve_type("TestEnum")
        .expect("Failed to resolve TestEnum")
        .expect("TestEnum type should be found");

    insta::assert_debug_snapshot!(test_enum_typedef);

    let (repr_c_typedef, _) = debug_info
        .resolve_type("ReprCEnum")
        .expect("Failed to resolve ReprCEnum")
        .expect("ReprCEnum type should be found");

    insta::assert_debug_snapshot!(repr_c_typedef);

    // we'll also test our special-cased enums Option and Result
    let (option_typedef, _) = debug_info
        .resolve_type("Option<i32>")
        .expect("Failed to resolve Option<i32>")
        .expect("Option<i32> type should be found");

    insta::assert_debug_snapshot!(option_typedef);

    let (result_typedef, _) = debug_info
        .resolve_type("Result<i32, String>")
        .expect("Failed to resolve Result<i32, String>")
        .expect("Result<i32, String> type should be found");

    insta::assert_debug_snapshot!(result_typedef);

    // Test U8Enum variants
    let (u8_enum_typedef, _) = debug_info
        .resolve_type("U8Enum")
        .expect("Failed to resolve U8Enum")
        .expect("U8Enum type should be found");

    insta::assert_debug_snapshot!(u8_enum_typedef);
}
