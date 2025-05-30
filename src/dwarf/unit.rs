use std::hash::Hash;

use gimli::{Unit, UnitSectionOffset};

use super::loader::DwarfReader;
use crate::database::Db;
use crate::dwarf::utils::get_dwarf;
use crate::file::File;

// // type MapUnitRef<'db> = crate::database::MappedRef<'db, UnitRef<'db>>;

// // type GimliUnitRef<'db> = gimli::UnitRef<'db, DwarfReader>;

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

unsafe impl<'db> salsa::Update for DwarfUnit {
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
    file: File,
    cu_offset: UnitSectionOffset<usize>,
) -> Option<UnitRef<'db, DwarfReader>> {
    #[salsa::tracked(returns(ref))]
    fn get_unit<'db>(
        db: &'db dyn Db,
        file: File,
        cu_offset: UnitSectionOffset<usize>,
    ) -> Option<DwarfUnit> {
        let dwarf = get_dwarf(db, file)?;

        let mut units = dwarf.units();
        while let Ok(Some(header)) = units.next() {
            if header.offset() == cu_offset {
                return Some(DwarfUnit {
                    file_path: file.path(db).to_string(),
                    offset: cu_offset,
                    unit: dwarf.unit(header).unwrap(),
                });
            }
        }
        None
    }

    let unit = &get_unit(db, file, cu_offset).as_ref()?.unit;
    let dwarf = get_dwarf(db, file)?;
    Some(UnitRef { dwarf, unit })
}

pub type UnitRef<'a, R = DwarfReader> = gimli::UnitRef<'a, R>;

// pub fn unit_ref<'db>(
//     dwarf: crate::database::MappedRef<'db, gimli::Dwarf<DwarfReader>>,
//     unit: &'db gimli::Unit<DwarfReader>,
// ) -> UnitRef<'db, DwarfReader> {
//     UnitRef { dwarf, unit }
// }
//
// /// This is akin to `UnitRef` from `gimli`, but with a mapped
// /// `Dwarf` reference that allows it to be used in salsa queries.
// // #[derive(Debug)]
// pub struct UnitRef<'a, R: gimli::Reader = DwarfReader> {
//     /// The `Dwarf` that contains the unit.
//     pub dwarf: &'a gimli::Dwarf<R>,

//     /// The `Unit` being referenced.
//     pub unit: &'a gimli::Unit<R>,
// }

// impl<'a, R: gimli::Reader> core::ops::Deref for UnitRef<'a, R> {
//     type Target = gimli::Unit<R>;

//     fn deref(&self) -> &Self::Target {
//         self.unit
//     }
// }

// mod imp {
//     use super::UnitRef;
//     use gimli::*;

//     impl<'a, R: gimli::Reader> UnitRef<'a, R> {
//         /// Return the string offset at the given index.
//         #[inline]
//         pub fn string_offset(
//             &self,
//             index: DebugStrOffsetsIndex<R::Offset>,
//         ) -> Result<DebugStrOffset<R::Offset>> {
//             self.dwarf.string_offset(self.unit, index)
//         }

//         /// Return the string at the given offset in `.debug_str`.
//         #[inline]
//         pub fn string(&self, offset: DebugStrOffset<R::Offset>) -> Result<R> {
//             self.dwarf.string(offset)
//         }

//         /// Return the string at the given offset in `.debug_line_str`.
//         #[inline]
//         pub fn line_string(&self, offset: DebugLineStrOffset<R::Offset>) -> Result<R> {
//             self.dwarf.line_string(offset)
//         }

//         /// Return the string at the given offset in the `.debug_str`
//         /// in the supplementary object file.
//         #[inline]
//         pub fn sup_string(&self, offset: DebugStrOffset<R::Offset>) -> Result<R> {
//             self.dwarf.sup_string(offset)
//         }

//         /// Return an attribute value as a string slice.
//         ///
//         /// See [`Dwarf::attr_string`] for more information.
//         pub fn attr_string(&self, attr: AttributeValue<R>) -> Result<R> {
//             self.dwarf.attr_string(self.unit, attr)
//         }

//         /// Return the address at the given index.
//         pub fn address(&self, index: DebugAddrIndex<R::Offset>) -> Result<u64> {
//             self.dwarf.address(self.unit, index)
//         }

//         /// Try to return an attribute value as an address.
//         ///
//         /// See [`Dwarf::attr_address`] for more information.
//         pub fn attr_address(&self, attr: AttributeValue<R>) -> Result<Option<u64>> {
//             self.dwarf.attr_address(self.unit, attr)
//         }

//         /// Return the range list offset for the given raw offset.
//         ///
//         /// This handles adding `DW_AT_GNU_ranges_base` if required.
//         pub fn ranges_offset_from_raw(
//             &self,
//             offset: RawRangeListsOffset<R::Offset>,
//         ) -> RangeListsOffset<R::Offset> {
//             self.dwarf.ranges_offset_from_raw(self.unit, offset)
//         }

//         /// Return the range list offset at the given index.
//         pub fn ranges_offset(
//             &self,
//             index: DebugRngListsIndex<R::Offset>,
//         ) -> Result<RangeListsOffset<R::Offset>> {
//             self.dwarf.ranges_offset(self.unit, index)
//         }

//         /// Iterate over the `RangeListEntry`s starting at the given offset.
//         pub fn ranges(&self, offset: RangeListsOffset<R::Offset>) -> Result<RngListIter<R>> {
//             self.dwarf.ranges(self.unit, offset)
//         }

//         /// Iterate over the `RawRngListEntry`ies starting at the given offset.
//         pub fn raw_ranges(&self, offset: RangeListsOffset<R::Offset>) -> Result<RawRngListIter<R>> {
//             self.dwarf.raw_ranges(self.unit, offset)
//         }

//         /// Try to return an attribute value as a range list offset.
//         ///
//         /// See [`Dwarf::attr_ranges_offset`] for more information.
//         pub fn attr_ranges_offset(
//             &self,
//             attr: AttributeValue<R>,
//         ) -> Result<Option<RangeListsOffset<R::Offset>>> {
//             self.dwarf.attr_ranges_offset(self.unit, attr)
//         }

//         /// Try to return an attribute value as a range list entry iterator.
//         ///
//         /// See [`Dwarf::attr_ranges`] for more information.
//         pub fn attr_ranges(&self, attr: AttributeValue<R>) -> Result<Option<RngListIter<R>>> {
//             self.dwarf.attr_ranges(self.unit, attr)
//         }

//         /// Return an iterator for the address ranges of a `DebuggingInformationEntry`.
//         ///
//         /// This uses `DW_AT_low_pc`, `DW_AT_high_pc` and `DW_AT_ranges`.
//         pub fn die_ranges(
//             &self,
//             entry: &DebuggingInformationEntry<'_, '_, R>,
//         ) -> Result<RangeIter<R>> {
//             self.dwarf.die_ranges(self.unit, entry)
//         }

//         /// Return an iterator for the address ranges of the `Unit`.
//         ///
//         /// This uses `DW_AT_low_pc`, `DW_AT_high_pc` and `DW_AT_ranges` of the
//         /// root `DebuggingInformationEntry`.
//         pub fn unit_ranges(&self) -> Result<RangeIter<R>> {
//             self.dwarf.unit_ranges(self.unit)
//         }

//         /// Return the location list offset at the given index.
//         pub fn locations_offset(
//             &self,
//             index: DebugLocListsIndex<R::Offset>,
//         ) -> Result<LocationListsOffset<R::Offset>> {
//             self.dwarf.locations_offset(self.unit, index)
//         }

//         /// Iterate over the `LocationListEntry`s starting at the given offset.
//         pub fn locations(&self, offset: LocationListsOffset<R::Offset>) -> Result<LocListIter<R>> {
//             self.dwarf.locations(self.unit, offset)
//         }

//         /// Iterate over the raw `LocationListEntry`s starting at the given offset.
//         pub fn raw_locations(
//             &self,
//             offset: LocationListsOffset<R::Offset>,
//         ) -> Result<RawLocListIter<R>> {
//             self.dwarf.raw_locations(self.unit, offset)
//         }

//         /// Try to return an attribute value as a location list offset.
//         ///
//         /// See [`Dwarf::attr_locations_offset`] for more information.
//         pub fn attr_locations_offset(
//             &self,
//             attr: AttributeValue<R>,
//         ) -> Result<Option<LocationListsOffset<R::Offset>>> {
//             self.dwarf.attr_locations_offset(self.unit, attr)
//         }

//         /// Try to return an attribute value as a location list entry iterator.
//         ///
//         /// See [`Dwarf::attr_locations`] for more information.
//         pub fn attr_locations(&self, attr: AttributeValue<R>) -> Result<Option<LocListIter<R>>> {
//             self.dwarf.attr_locations(self.unit, attr)
//         }
//     }
// }
