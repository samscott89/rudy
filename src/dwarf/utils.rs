//! DWARF parsing utilities and helper functions

use anyhow::Result;
use gimli::Reader;
use itertools::Itertools;

use super::loader::{RawDie, DwarfReader, UnitRef};

pub fn get_string_attr_raw<'a>(
    die: &RawDie<'a>,
    attr: gimli::DwAt,
    unit_ref: &UnitRef<'a>,
) -> Result<Option<DwarfReader>> {
    let attr = die.attr(attr)?;
    let Some(attr) = attr else { return Ok(None) };
    let value = attr.value();
    let value = unit_ref.attr_string(value)?;
    Ok(Some(value))
}

pub fn get_string_attr<'a>(
    die: &RawDie<'a>,
    attr: gimli::DwAt,
    unit_ref: &UnitRef<'a>,
) -> Result<Option<String>> {
    let value = get_string_attr_raw(die, attr, unit_ref)?;
    let Some(value) = value else { return Ok(None) };
    let value = value.to_string()?;
    Ok(Some(value.into_owned()))
}

pub fn parse_die_string_attribute<'a>(
    die: &RawDie<'a>,
    attr: gimli::DwAt,
    unit_ref: &UnitRef<'a>,
) -> Result<Option<String>> {
    let attr = die.attr(attr)?;
    let Some(attr) = attr else { return Ok(None) };
    let value = attr.value();
    let value = unit_ref.attr_string(value)?;
    Ok(Some(value.to_string()?.into_owned()))
}

pub fn file_entry_to_path(f: &gimli::FileEntry<DwarfReader>, unit_ref: &UnitRef) -> Option<String> {
    let lp = unit_ref.line_program.as_ref()?;
    let header = lp.header();
    let dir = f.directory(header)?;
    let dir = unit_ref.attr_string(dir).ok()?;
    let path = unit_ref.attr_string(f.path_name()).ok()?;
    Some(format!(
        "{}/{}",
        dir.to_string().ok()?,
        path.to_string().ok()?
    ))
}

pub fn to_range(mut iter: gimli::RangeIter<DwarfReader>) -> Result<Option<(u64, u64)>> {
    let mut lowest_pc = None;
    let mut highest_pc = None;

    while let Some(range) = iter.next()? {
        if lowest_pc.is_none_or(|lowest_pc| range.begin < lowest_pc) {
            lowest_pc = Some(range.begin);
        }
        if highest_pc.is_none_or(|highest_pc| range.end > highest_pc) {
            highest_pc = Some(range.end);
        }
    }
    Ok(lowest_pc.zip(highest_pc))
}

const MAX_DIE_ATTR_LENGTH: usize = 512;

pub fn debug_print_die_entry(die: &RawDie<'_>) -> String {
    let mut attrs = die.attrs();
    let attrs_iter = std::iter::from_fn(|| attrs.next().ok().flatten());
    let attrs = attrs_iter
        .map(|a| {
            let name = a.name();
            let value = a.value();

            let string_value = format!("{value:?}");
            if string_value.len() > MAX_DIE_ATTR_LENGTH {
                format!("{name}: {}..", &string_value[..MAX_DIE_ATTR_LENGTH])
            } else {
                format!("{name}: {string_value}")
            }
        })
        .join(",\n\t");
    format!("{:#x} {} {{\n\t{}\n}}", die.offset().0, die.tag(), attrs)
}

pub fn pretty_print_die_entry(die: &RawDie<'_>, unit_ref: &UnitRef<'_>) -> String {
    let mut attrs = die.attrs();
    let attrs_iter = std::iter::from_fn(|| attrs.next().ok().flatten());
    let attrs = attrs_iter
        .map(|a| {
            let name = a.name();
            let value = a.value();

            if let Ok(v) = unit_ref.attr_string(a.value()) {
                format!(
                    "{name}: {}",
                    v.to_string_lossy()
                        .map_or_else(|_| format!("{value:?}"), |v| v.into_owned())
                )
            } else {
                let string_value = format!("{value:?}");
                if string_value.len() > MAX_DIE_ATTR_LENGTH {
                    format!("{name}: {}..", &string_value[..MAX_DIE_ATTR_LENGTH])
                } else {
                    format!("{name}: {string_value}")
                }
            }
        })
        .join(",\n\t");
    format!("{} {{\n\t{}\n}}", die.tag(), attrs)
}