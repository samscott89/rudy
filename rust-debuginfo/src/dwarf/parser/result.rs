//! Result parser implementation using combinators

use super::Parser;
use crate::dwarf::parser::enums::enum_def;
use rust_types::{EnumDef, ResultDef};

use anyhow::Context as _;

/// Parser for result types
///
/// We'll parse it as a generic enum, and extract out the expect "Some" variant
pub fn result_def<'db>() -> impl Parser<'db, ResultDef> {
    enum_def().map_res(
        |EnumDef {
             name,
             discriminant,
             variants,
             size,
         }| {
            let ok_type = variants
                .iter()
                .find(|v| v.name == "Ok")
                .map(|v| v.layout.clone())
                .context("No Ok type found for Result enum")?;
            let err_type = variants
                .iter()
                .find(|v| v.name == "Err")
                .map(|v| v.layout.clone())
                .context("No Err type found for Result enum")?;
            Ok(ResultDef {
                name,
                discriminant,
                ok_type,
                err_type,
                size,
            })
        },
    )
}
