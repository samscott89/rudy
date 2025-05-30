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
    ($target:ident) => {
        let _ = tracing_subscriber::fmt::try_init();
        let mut settings = insta::Settings::clone_current();

        // get current OS as a prefix
        settings.set_snapshot_suffix($target);
        settings.set_prepend_module_to_snapshot(false);

        let _guard = settings.bind_to_scope();
        let test_name = crate::function_name!();
        let test_name = test_name
            .strip_prefix("rust_debuginfo::")
            .unwrap_or(test_name);
        let test_name = test_name.strip_prefix("tests::").unwrap_or(test_name);
        let _span = tracing::info_span!("test", test_name, $target).entered();
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

#[apply(binary_target)]
fn test_resolve_function(#[case] target: &str) {
    setup!(target);
    let path = binary_path(target);
    let db = crate::DebugDb::new().unwrap();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(resolver.resolve_function("main").unwrap());
    insta::assert_debug_snapshot!(resolver.resolve_function("function_call").unwrap());
}

#[apply(binary_target)]
fn test_resolve_position(#[case] target: &str) {
    setup!(target);
    let path = binary_path(target);
    let db = crate::DebugDb::new().unwrap();
    let resolver = DebugInfo::new(&db, &path).unwrap();

    // should be the position of the `let y = x + 1;` line
    let addrs = resolver
        .resolve_position("examples/simple-test/src/main.rs", 4, None)
        .unwrap()
        .unwrap();
    insta::assert_debug_snapshot!(addrs);
    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "function_call".to_string(),
            file: "examples/simple-test/src/main.rs".to_string(),
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
        .resolve_position("examples/simple-test/src/main.rs", 16, None)
        .unwrap()
        .unwrap();
    // address of line 17
    insta::assert_debug_snapshot!(addrs);

    assert_eq!(
        resolver.address_to_line(addrs.address).unwrap(),
        ResolvedLocation {
            function: "main".to_string(),
            file: "examples/simple-test/src/main.rs".to_string(),
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
    //         file: "examples/simple-test/src/main.rs".to_string(),
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
    let db = crate::DebugDb::new().unwrap();
    let parsed = DebugInfo::new(&db, &path).unwrap();

    insta::assert_debug_snapshot!(parsed);
}
