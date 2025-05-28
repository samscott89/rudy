//! DWARF loading and memory management

use std::{
    ops::Deref,
    sync::Arc,
};

use gimli::{EndianReader, LittleEndian};
use object::ObjectSection;

// Utilities for loading dwarf files

pub fn load(file: &'_ object::File<'static>) -> gimli::Result<Dwarf> {
    let load = |id: gimli::SectionId| {
        Ok(gimli::EndianReader::new(
            object::Object::section_by_name(file, id.name())
                .map(|s| match s.uncompressed_data().unwrap() {
                    std::borrow::Cow::Borrowed(b) => OwnedOrBorrowed::Borrowed(b),
                    std::borrow::Cow::Owned(v) => OwnedOrBorrowed::Owned(Arc::new(v)),
                })
                .unwrap_or(OwnedOrBorrowed::Borrowed(&[])),
            gimli::LittleEndian,
        ))
    };
    gimli::Dwarf::load(&load)
}

#[derive(Clone, Debug)]
pub enum OwnedOrBorrowed {
    Borrowed(&'static [u8]),
    Owned(Arc<Vec<u8>>),
}

// And `OwnedOrBorrowed` can deref to a slice of the `mmap`ed region of memory.
impl<'a> Deref for OwnedOrBorrowed {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            OwnedOrBorrowed::Borrowed(slice) => slice,
            OwnedOrBorrowed::Owned(arc) => &**arc,
        }
    }
}

// These are both valid for any `Rc` or `Arc`.
unsafe impl gimli::StableDeref for OwnedOrBorrowed {}
unsafe impl gimli::CloneStableDeref for OwnedOrBorrowed {}

// shorthand type definitions
pub type DwarfReader = EndianReader<LittleEndian, OwnedOrBorrowed>;
pub type Dwarf = gimli::Dwarf<DwarfReader>;
pub type UnitRef<'a> = gimli::UnitRef<'a, DwarfReader>;
pub type Die<'a> = gimli::DebuggingInformationEntry<'a, 'a, DwarfReader>;
pub type Offset = gimli::UnitOffset<usize>;