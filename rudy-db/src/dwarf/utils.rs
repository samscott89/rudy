//! DWARF parsing utilities and helper functions

use anyhow::{Context, Result};
use gimli::Reader;
use itertools::Itertools;

use crate::file::File;

use super::{
    loader::{DwarfReader, Offset, RawDie},
    unit::UnitRef,
};

pub fn get_dwarf(db: &dyn crate::database::Db, file: File) -> Option<&super::Dwarf> {
    let loaded = crate::file::load(db, file).as_ref().ok()?;
    Some(&loaded.dwarf)
}

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

pub fn get_lang_attr<'a>(
    die: &RawDie<'a>,
    _unit_ref: &UnitRef<'a>,
) -> Result<Option<gimli::DwLang>> {
    let attr = die.attr(gimli::DW_AT_language)?;
    let Some(attr) = attr else { return Ok(None) };
    let value = attr.value();
    let gimli::AttributeValue::Language(lang) = value else {
        return Ok(None);
    };
    Ok(Some(lang))
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

pub fn get_unit_ref_attr<'a>(die: &RawDie<'a>, attr: gimli::DwAt) -> Result<Offset> {
    let value = die
        .attr_value(attr)
        .with_context(|| format!("Failed to get `{attr}` attribute"))?
        .with_context(|| format!("attribute `{attr}` not found"))?;
    if let gimli::AttributeValue::UnitRef(offset) = value {
        Ok(offset)
    } else {
        Err(anyhow::anyhow!(
            "Expected UnitRef attribute for {:#x}, got {value:?}",
            die.offset().0,
        ))
    }
}

pub fn parse_die_string_attribute<'a>(
    die: &RawDie<'a>,
    attr: gimli::DwAt,
    unit_ref: &UnitRef<'a>,
) -> Result<String> {
    let attr = die
        .attr(attr)?
        .with_context(|| format!("Failed to get string attribute `{attr}`"))?;
    let value = attr.value();
    let value = unit_ref.attr_string(value)?;
    Ok(value.to_string()?.into_owned())
}

pub fn file_entry_to_path<'db>(
    f: &gimli::FileEntry<DwarfReader>,
    unit_ref: &UnitRef<'db>,
) -> Option<String> {
    let lp = unit_ref
        .line_program
        .as_ref()
        .expect("Line program should be present");
    let header = lp.header();
    let dir = f
        .directory(header)
        .and_then(|d| unit_ref.attr_string(d).ok());
    let dir = dir.as_ref().and_then(|d| d.to_string().ok());
    let path = unit_ref
        .attr_string(f.path_name())
        .inspect_err(|e| {
            tracing::debug!("Failed to convert path to string: {e}");
        })
        .ok()?;
    let path = path
        .to_string()
        .inspect_err(|e| {
            tracing::debug!("Failed to convert path to string: {e}");
        })
        .ok()?;

    if let Some(d) = dir {
        if !d.starts_with("/") {
            // this is a relative path, so we need to prepend the current working directory
            let compilation_dir = unit_ref.comp_dir.as_ref().map_or_else(
                || "/".to_string(),
                |d| {
                    d.to_string()
                        .ok()
                        .map_or_else(|| "/".to_string(), |d| d.into_owned())
                },
            );
            Some(format!("{compilation_dir}/{d}/{path}"))
        } else {
            Some(format!("{d}/{path}"))
        }
    } else {
        Some(path.to_string())
    }
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
