//! Static tests that analyze pre-built binaries
//! These tests can run on any platform as they only read debug info

use rstest::rstest;
use rstest_reuse::{self, *};
use rudy_db::*;

#[macro_use]
pub mod common;

use common::artifacts_dir;

pub fn binary_path(target: &str, example: &str) -> String {
    let artifacts = artifacts_dir();
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

pub fn platform_source_file(_target: &str, file: &str) -> String {
    // For test artifacts, we'll use a consistent path structure
    format!("/test/{file}")
}

#[template]
#[rstest]
#[case("aarch64-unknown-linux-gnu")]
#[case("x86_64-unknown-linux-gnu")]
#[case("aarch64-apple-darwin")]
#[case("x86_64-apple-darwin")]
pub fn binary_target(#[case] target: &str) {}

#[apply(binary_target)]
fn test_resolve_function(#[case] target: &str) {
    let _guards = setup!(target);
    let path = binary_path(target, "simple_test");
    let db = crate::DebugDb::new();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(resolver.resolve_function("main").unwrap());
    insta::assert_debug_snapshot!(resolver.resolve_function("function_call").unwrap());
}

#[apply(binary_target)]
fn test_resolve_position(#[case] target: &str) {
    let _guards = setup!(target);
    let path = binary_path(target, "simple_test");
    let db = crate::DebugDb::new();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    let platform_file = platform_source_file(target, "simple_test.rs");

    // should be the position of the `let y = x + 1;` line
    let addrs = resolver
        .resolve_position(&platform_file, 4, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);
    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "simple_test::function_call".to_string(),
            file: platform_file.clone(),
            line: 4,
        }
    );

    // should be the position of the `const Z: u64 = 0xdeadbeef;` line
    let addrs = resolver
        .resolve_position(&platform_file, 16, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);

    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "simple_test::main".to_string(),
            file: platform_file.clone(),
            line: 17,
        }
    );
}

#[apply(binary_target)]
fn test_load_file(#[case] target: &str) {
    let _guards = setup!(target);

    let path = binary_path(target, "simple_test");

    let db = crate::DebugDb::new();
    let parsed = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(parsed);
}
