use std::path::{Path, PathBuf};

// re-export all test utilities for easy access from tests
pub use ::test_utils::*;

#[salsa::db]
#[derive(Clone)]
pub struct TestDatabaseImpl {
    storage: salsa::Storage<Self>,
    source_map: Vec<(PathBuf, PathBuf)>,
}

#[salsa::db]
impl salsa::Database for TestDatabaseImpl {}

#[salsa::db]
impl rudy_dwarf::DwarfDb for TestDatabaseImpl {
    fn get_source_map(&self) -> &[(PathBuf, PathBuf)] {
        &self.source_map
    }
}

pub fn test_db(target: Option<&'static str>) -> TestDatabaseImpl {
    let source_map = source_map(target);
    TestDatabaseImpl {
        storage: salsa::Storage::default(),
        source_map,
    }
}

pub fn load_binary<P: AsRef<Path>>(
    db: &dyn rudy_dwarf::DwarfDb,
    path: P,
) -> rudy_dwarf::file::Binary {
    let test_binary = rudy_dwarf::file::File::build(db, path.as_ref().into(), None).unwrap();
    rudy_dwarf::file::Binary::new(db, test_binary)
}
