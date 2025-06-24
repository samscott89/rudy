use rust_debuginfo::*;

use rstest::rstest;
use rstest_reuse::{self, *};

#[macro_export]
macro_rules! function_name {
    () => {{
        // Okay, this is ugly, I get it. However, this is the best we can get on a stable rust.
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        // `3` is the length of the `::f`.
        &name[..name.len() - 3]
    }};
}

#[macro_export]
macro_rules! setup {
    ($($target:ident)?) => {
        let _ = tracing_subscriber::fmt::try_init();
        let mut settings = insta::Settings::clone_current();

        // get current OS as a prefix
        $(
            settings.set_snapshot_suffix($target);
        )?
        settings.set_prepend_module_to_snapshot(false);

        let _guard = settings.bind_to_scope();
        let test_name = crate::function_name!();
        let test_name = test_name
            .strip_prefix("rust_debuginfo::")
            .unwrap_or(test_name);
        let test_name = test_name.strip_prefix("tests::").unwrap_or(test_name);
        let _span = tracing::info_span!("test", test_name, $($target)?).entered();
    };
}

#[template]
#[rstest]
#[case("aarch64-unknown-linux-gnu")]
#[case("x86_64-unknown-linux-gnu")]
#[case("aarch64-apple-darwin")]
#[case("x86_64-apple-darwin")]
pub fn binary_target(#[case] target: &str) {}

pub fn binary_path(target: &str) -> String {
    let workspace_dir = std::env::var("CARGO_WORKSPACE_DIR").unwrap();
    format!("{workspace_dir}target/{target}/debug-test/simple-test")
}

pub fn platform_source_file(target: &str, file: &str) -> String {
    match target {
        "aarch64-unknown-linux-gnu" | "x86_64-unknown-linux-gnu" => {
            format!("/app/{file}")
        }
        "aarch64-apple-darwin" | "x86_64-apple-darwin" => {
            format!("/Users/sam/work/tardis/{file}")
        }
        _ => panic!("Unsupported target: {target}"),
    }
}

#[apply(binary_target)]
fn test_resolve_function(#[case] target: &str) {
    setup!(target);
    let path = binary_path(target);
    let db = crate::DebugDb::new();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(resolver.resolve_function("main").unwrap());
    insta::assert_debug_snapshot!(resolver.resolve_function("function_call").unwrap());
}

#[apply(binary_target)]
fn test_resolve_position(#[case] target: &str) {
    setup!(target);
    let path = binary_path(target);
    let db = crate::DebugDb::new();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    let platform_file = platform_source_file(target, "examples/simple-test/src/main.rs");

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
    // however, this gets inlined and so we end up at the next line
    //
    // same behaviour as LLDB:
    //   (lldb) b simple-test.rs:21
    //   Breakpoint 4: where = simple-test`simple_test::main::h8787f5d764ea460c + 20 at simple-test.rs:22:5, address = 0x00000001000041a0``
    let addrs = resolver
        .resolve_position(&platform_file, 16, None)
        .unwrap()
        .unwrap();
    // address of line 17
    insta::assert_debug_snapshot!(addrs);

    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "simple_test::main".to_string(),
            file: platform_file.clone(),
            // This is right: it resolves to the next line cause
            // that's closest line that has an instruction
            line: 17,
        }
    );

    // TODO: come up with a way to test this (stack frames)
    // assert_eq!(
    //     resolver.address_to_line(0x100003948).unwrap(),
    //     ResolvedLocation {
    //         function: "main".to_string(),
    //         file: platform_file.clone(),
    //         // TODO(Sam): this is wrong. This should be line 18
    //         // based on the output of LLDB.
    //         // this is currently pointing to the end of the function rather
    //         // than the function call.
    //         // suspect we need to track "end of statement" smarter or something
    //         line: 19,
    //     }
    // );
}

#[apply(binary_target)]
fn test_load_file(#[case] target: &str) {
    setup!(target);

    let path = binary_path(target);
    let db = crate::DebugDb::new();
    let parsed = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(parsed);
}

#[rstest]
#[case("small")]
#[case("medium")]
#[case("large")]
fn test_generated_benchmarks(#[case] target: &str) {
    setup!(target);
    let path = format!("bin/test_binaries/{target}");

    if !std::fs::exists(&path).unwrap() {
        panic!(
            "Please run `cargo run --bin generate_test_binaries` to generate the test binaries first."
        );
    }

    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(debug_info);

    // resolve functions
    insta::assert_debug_snapshot!(debug_info.resolve_function("main").unwrap().unwrap());
    insta::assert_debug_snapshot!(
        debug_info
            .resolve_function("TestStruct0::method_0")
            .unwrap()
            .unwrap()
    );
    insta::assert_debug_snapshot!(
        debug_info
            .resolve_function("TestStruct1::method_1")
            .unwrap()
            .unwrap()
    );

    let f = debug_info
        .resolve_function("TestStruct0::method_0")
        .unwrap()
        .unwrap();

    // resolve the address of the function
    let location = debug_info
        .resolve_address_to_location(f.address)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(location);

    // resolve positions
    let addrs = debug_info
        .resolve_position(&format!("{target}.rs"), location.line, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);
}

// TODO: add a test that we correctly resolve inlined functions

#[rstest]
#[case("small")]
fn test_method_discovery(#[case] target: &str) {
    setup!(target);
    let path = format!("bin/test_binaries/{target}");

    if !std::fs::exists(&path).unwrap() {
        panic!(
            "Please run `cargo run --bin generate_test_binaries` to generate the test binaries first."
        );
    }

    let db = DebugDb::new();
    let debug_info = DebugInfo::new(&db, &path).unwrap();

    // Test 1: Discover all methods in the binary
    let methods_by_type = debug_info
        .discover_all_methods()
        .expect("Method discovery should succeed");

    insta::assert_debug_snapshot!(methods_by_type);

    // Test 2: Test specific type resolution and method discovery
    let test_struct0_type = debug_info
        .resolve_type("TestStruct0")
        .expect("Type resolution should succeed")
        .expect("TestStruct0 type should be found");

    let methods = debug_info
        .discover_methods_for_type(&test_struct0_type)
        .expect("Method discovery for TestStruct0 should succeed");

    insta::assert_debug_snapshot!(methods);
}
