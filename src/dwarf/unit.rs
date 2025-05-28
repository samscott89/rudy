use std::{hash::Hash, sync::Arc};

use gimli::{Unit, UnitSectionOffset};

use super::loader::{Dwarf, DwarfReader, UnitRef};
use crate::database::Db;
use crate::file::FileId;

#[derive(Clone, Debug)]
struct DwarfUnit {
    file_path: Arc<String>,
    offset: UnitSectionOffset<usize>,
    unit: Arc<Unit<DwarfReader>>,
}

impl DwarfUnit {
    fn unit_ref<'a>(&'a self, dwarf: &'a Dwarf) -> UnitRef<'a> {
        self.unit.unit_ref(dwarf)
    }
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

unsafe impl<'db> salsa::Update for DwarfUnit {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        // Because everything is owned, this ref is simply a valid `&mut`
        let old_ref: &mut Self = unsafe { &mut *old_pointer };

        let mut changed = false;

        if old_ref.file_path != new_value.file_path {
            changed = true;
            old_ref.file_path = new_value.file_path;
        }

        if old_ref.offset != new_value.offset {
            changed = true;
            old_ref.offset = new_value.offset;
        }

        if !Arc::ptr_eq(&old_ref.unit, &new_value.unit) {
            changed = true;
            old_ref.unit = new_value.unit.clone();
        }

        changed
    }
}

/// Get's a `UnitRef` for a given compilation unit offset
///
/// `UnitRef`s are a little unwieldy since their lifetime
/// is tied to the `Unit` they are created from, and so
/// _normally_ we can't just return them from a funciton
///
/// By making this a `salsa::tracked` function, the underlying
/// `Unit` is "stored" in the salsa database, and so we can
/// can get a reference to with a lifetime of `'db` which
/// _then_ means that we can return a `UnitRef` with a lifetime
/// of `'db` as well.
pub fn get_unit_ref<'db>(
    db: &'db dyn Db,
    file_id: FileId<'db>,
    cu_offset: UnitSectionOffset<usize>,
) -> Option<UnitRef<'db>> {
    #[salsa::tracked(return_ref)]
    fn get_unit<'db>(
        db: &'db dyn Db,
        file_id: FileId<'db>,
        cu_offset: UnitSectionOffset<usize>,
    ) -> Option<DwarfUnit> {
        let Some(file) = db.get_file(file_id) else {
            return None;
        };
        let Some(dwarf) = file.dwarf() else {
            return None;
        };

        let mut units = dwarf.units();
        while let Ok(Some(header)) = units.next() {
            if header.offset() == cu_offset {
                return Some(DwarfUnit {
                    file_path: Arc::new(file_id.full_path(db).to_string()),
                    offset: cu_offset,
                    unit: Arc::new(dwarf.unit(header).unwrap()),
                });
            }
        }
        None
    }

    let Some(unit) = get_unit(db, file_id, cu_offset) else {
        return None;
    };
    let Some(dwarf) = db.get_file(file_id).and_then(|f| f.dwarf()) else {
        return None;
    };
    Some(unit.unit_ref(dwarf))
}
