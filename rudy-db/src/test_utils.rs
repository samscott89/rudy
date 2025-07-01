// re-export all test utilities for easy access from tests
pub use ::test_utils::*;

pub fn debug_db(target: Option<&'static str>) -> crate::DebugDb {
    crate::DebugDb::new().with_source_map(source_map(target))
}
