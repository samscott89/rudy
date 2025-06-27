use std::hash::Hash;

use gimli::{Unit, UnitSectionOffset};

use super::loader::DwarfReader;
use crate::database::Db;
use crate::dwarf::utils::get_dwarf;
use crate::file::DebugFile;

pub type UnitRef<'a, R = DwarfReader> = gimli::UnitRef<'a, R>;

#[derive(Debug)]
struct DwarfUnit {
    file_path: String,
    offset: UnitSectionOffset<usize>,
    unit: Unit<DwarfReader>,
}

impl PartialEq for DwarfUnit {
    fn eq(&self, other: &Self) -> bool {
        self.file_path == other.file_path && self.offset == other.offset
    }
}

impl Eq for DwarfUnit {}

impl Hash for DwarfUnit {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.file_path.hash(state);
        self.offset.hash(state);
    }
}

unsafe impl salsa::Update for DwarfUnit {
    unsafe fn maybe_update(_: *mut Self, _: Self) -> bool {
        // DwarfUnits should _never_ change
        false
    }
}

/// Get's a `UnitRef` for a given compilation unit offset
///
/// `UnitRef`s are a little unwieldy since their lifetime
/// is tied to the `Unit` they are created from, and so
/// _normally_ we can't just return them from a funciton
///
/// By making `get_unit` a `salsa::tracked` function, the underlying
/// `Unit` is "stored" in the salsa database, and so we can
/// can get a reference to with a lifetime of `'db` which
/// _then_ means that we can return a `UnitRef` with a lifetime
/// of `'db` as well.
pub fn get_unit_ref<'db>(
    db: &'db dyn Db,
    file: DebugFile,
    cu_offset: UnitSectionOffset<usize>,
) -> Option<UnitRef<'db, DwarfReader>> {
    #[salsa::tracked(returns(ref))]
    fn get_unit<'db>(
        db: &'db dyn Db,
        file: DebugFile,
        cu_offset: UnitSectionOffset<usize>,
    ) -> Option<DwarfUnit> {
        let dwarf = get_dwarf(db, file.file(db))?;

        let mut units = dwarf.units();
        while let Ok(Some(header)) = units.next() {
            if header.offset() == cu_offset {
                return Some(DwarfUnit {
                    file_path: file.file(db).path(db).to_string(),
                    offset: cu_offset,
                    unit: dwarf.unit(header).unwrap(),
                });
            }
        }
        None
    }

    let unit = &get_unit(db, file, cu_offset).as_ref()?.unit;
    let dwarf = get_dwarf(db, file.file(db))?;
    Some(UnitRef { dwarf, unit })
}
