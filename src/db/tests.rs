use rstest::rstest;
use rstest_reuse::{self, *};
use salsa::Database;

use super::*;
use crate::setup;
use crate::tests::{binary_path, binary_target};

#[apply(binary_target)]
fn test_lookup_function(#[case] target: &str) {
    setup!(target);
    let binary_path = binary_path(target);
    let linkage_name = DebugDatabaseImpl::new(&binary_path).unwrap().attach(|db| {
        let f = db.lookup_function("function_call").unwrap();
        insta::assert_debug_snapshot!(f);

        f.unwrap().linkage_name(db).clone()
    });

    assert!(
        linkage_name.contains("function_call"),
        "got: {linkage_name}"
    );
}

#[apply(binary_target)]
fn test_index(#[case] target: &str) {
    setup!(target);
    let binary_path = binary_path(target);
    DebugDatabaseImpl::new(&binary_path).unwrap().attach(|db| {
        let index = super::index(db).data(db);
        insta::assert_debug_snapshot!(index);
    });
}

#[apply(binary_target)]
fn test_address_to_line(#[case] target: &str) {
    setup!(target);
    let binary_path = binary_path(target);
    DebugDatabaseImpl::new(&binary_path).unwrap().attach(|db| {
        let loc = db
            .resolve_position("examples/simple-test/src/main.rs", 4, None)
            .unwrap()
            .unwrap();

        let f = db.resolve_address_to_location(loc.address).unwrap();
        insta::assert_debug_snapshot!(f);
    });
}

struct TestDataResolver;

impl crate::DataResolver for TestDataResolver {
    fn base_address(&self) -> u64 {
        0x00000000
    }

    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        if size == 0 {
            return Ok(vec![]);
        }
        tracing::debug!("read_memory {address:#x} {size}");
        Ok(std::iter::once((address & 0xff) as u8)
            .chain(std::iter::repeat_n(0, size - 1))
            .collect())
    }

    fn get_registers(&self) -> Result<Vec<u64>> {
        tracing::debug!("get_registers");
        Ok((0..64).map(|i| i).collect())
    }
}

#[apply(binary_target)]
fn test_resolve_function_variables(#[case] target: &str) {
    setup!(target);
    let binary_path = binary_path(target);
    DebugDatabaseImpl::new(&binary_path).unwrap().attach(|db| {
        let loc = db
            .resolve_position("examples/simple-test/src/main.rs", 4, None)
            .unwrap()
            .unwrap();
        let f = db
            .resolve_variables_at_address(loc.address, &TestDataResolver)
            .unwrap();
        insta::assert_debug_snapshot!(f);
    });

    // expected:
    /*
    0x00000078:     DW_TAG_subprogram
                      DW_AT_low_pc	(0x0000000000000000)
                      DW_AT_high_pc	(0x0000000000000110)
                      DW_AT_frame_base	(DW_OP_reg29 W29)
                      DW_AT_linkage_name	("_ZN11simple_test13function_call17hc3f3f8a282d9d107E")
                      DW_AT_name	("function_call")
                      DW_AT_decl_file	("/Users/sam/work/tardis/examples/simple-test/src/main.rs")
                      DW_AT_decl_line	(8)

    0x00000091:       DW_TAG_formal_parameter
                        DW_AT_location	(DW_OP_fbreg -24)
                        DW_AT_name	("x")
                        DW_AT_decl_file	("/Users/sam/work/tardis/examples/simple-test/src/main.rs")
                        DW_AT_decl_line	(8)
                        DW_AT_type	(0x000000cf "u64")

    0x0000009f:       DW_TAG_lexical_block
                        DW_AT_ranges	(0x00000000
                           [0x0000000000000038, 0x000000000000007c)
                           [0x0000000000000088, 0x0000000000000100))

    0x000000a4:         DW_TAG_variable
                          DW_AT_location	(DW_OP_breg31 WSP+40)
                          DW_AT_name	("y")
                          DW_AT_alignment	(1)
                          DW_AT_decl_file	("/Users/sam/work/tardis/examples/simple-test/src/main.rs")
                          DW_AT_decl_line	(9)
                          DW_AT_type	(0x000000cf "u64")

    0x000000b3:         NULL

    0x000000b4:       NULL
    */
}

#[apply(binary_target)]
fn test_get_shape(#[case] target: &str) {
    setup!(target);
    let binary_path = binary_path(target);
    DebugDatabaseImpl::new(&binary_path).unwrap().attach(|db| {
        let def = db.test_get_shape().unwrap();
        insta::assert_debug_snapshot!(def);
    });
}
