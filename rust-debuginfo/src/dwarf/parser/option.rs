//! Option parser implementation using combinators

use super::Parser;
use crate::dwarf::parser::enums::enum_def;
use rust_types::{EnumDef, OptionDef};

use anyhow::Context as _;

/// Parser for option types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn option_def<'db>() -> impl Parser<'db, OptionDef> {
    enum_def().map_res(
        |EnumDef {
             name,
             discriminant,
             variants,
             size,
         }| {
            let some_type = variants
                .iter()
                .find(|v| v.name == "Some")
                .map(|v| v.layout.clone())
                .context("No Some type found for Option enum")?;
            Ok(OptionDef {
                name,
                discriminant,
                some_type,
                size,
            })
        },
    )
}
