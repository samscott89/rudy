//! Simplified parser combinator framework for DWARF type resolution
//!
//! This demonstrates a cleaner approach to composable DWARF parsing that solves
//! the brittleness and reusability issues in the current type resolution code.

use std::sync::Arc;
use crate::database::Db;
use crate::dwarf::Die;
use rust_types::*;

type Result<T> = std::result::Result<T, super::Error>;

/// Core parser trait
pub trait Parser<'db, T> {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<T>;
}

/// Parse a field by name and return its Die
pub struct Field {
    name: String,
}

impl<'db> Parser<'db, Die<'db>> for Field {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry.get_member(db, &self.name)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to find field '{}': {}", self.name, e)))
    }
}

/// Parse field offset
pub struct FieldOffset {
    field_name: String,
}

impl<'db> Parser<'db, usize> for FieldOffset {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<usize> {
        entry.get_udata_member_attribute(db, &self.field_name, gimli::DW_AT_data_member_location)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get offset for field '{}': {}", self.field_name, e)))
            .map(|offset| offset as usize)
    }
}

/// Parse a generic type parameter
pub struct Generic {
    param_name: String,
}

impl<'db> Parser<'db, Die<'db>> for Generic {
    fn parse(&self, db: &'db dyn Db, entry: Die<'db>) -> Result<Die<'db>> {
        entry.get_generic_type_entry(db, &self.param_name)
            .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to resolve generic parameter '{}': {}", self.param_name, e)))
    }
}

/// Parse field path and accumulate offsets
pub struct FieldPath {
    path: Vec<String>,
}

impl<'db> Parser<'db, (Die<'db>, usize)> for FieldPath {
    fn parse(&self, db: &'db dyn Db, mut entry: Die<'db>) -> Result<(Die<'db>, usize)> {
        let mut total_offset = 0;
        
        for field_name in &self.path {
            let offset = entry.get_udata_member_attribute(db, field_name, gimli::DW_AT_data_member_location)
                .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to get offset for field '{}': {}", field_name, e)))?;
            total_offset += offset as usize;
            
            entry = entry.get_member(db, field_name)
                .map_err(|e| super::Error::from(anyhow::anyhow!("Failed to navigate to field '{}': {}", field_name, e)))?;
        }
        
        Ok((entry, total_offset))
    }
}

/// Helper functions
pub fn field(name: &str) -> Field {
    Field { name: name.to_string() }
}

pub fn field_offset(name: &str) -> FieldOffset {
    FieldOffset { field_name: name.to_string() }
}

pub fn generic(name: &str) -> Generic {
    Generic { param_name: name.to_string() }
}

pub fn field_path(path: &[&str]) -> FieldPath {
    FieldPath { 
        path: path.iter().map(|&s| s.to_string()).collect() 
    }
}

/// Vec parser using the combinator approach
pub fn parse_vec_layout<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<VecDef> {
    // Parse data pointer offset using field path
    let data_ptr_offset = field_path(&["buf", "inner", "ptr"]).parse(db, entry)?.1;
    
    // Parse length offset
    let length_offset = field_offset("len").parse(db, entry)?;
    
    // Parse inner type
    let inner_type_die = generic("T").parse(db, entry)?;
    let inner_type = super::shallow_resolve_type(db, inner_type_die)?;
    
    Ok(VecDef {
        data_ptr_offset,
        length_offset,
        inner_type: Arc::new(inner_type),
    })
}

/// Option parser - reusable for BTreeMap
pub fn parse_option_layout<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<EnumDef> {
    // For now, we'd need to make resolve_enum_type public or reimplement
    // This shows the concept - we can reuse enum parsing logic
    todo!("Need to expose resolve_enum_type or reimplement here")
}

/// Example showing how BTreeMap could reuse Option parsing
pub fn parse_btreemap_layout<'db>(db: &'db dyn Db, entry: Die<'db>) -> Result<MapDef> {
    // Get basic info
    let key_type_die = generic("K").parse(db, entry)?;
    let key_type = Arc::new(super::shallow_resolve_type(db, key_type_die)?);
    
    let value_type_die = generic("V").parse(db, entry)?;
    let value_type = Arc::new(super::shallow_resolve_type(db, value_type_die)?);
    
    let length_offset = field_offset("length").parse(db, entry)?;
    
    // Get root field which is Option<NodeRef>
    let root_field = field("root").parse(db, entry)?;
    let root_offset = field_offset("root").parse(db, entry)?;
    
    // Parse the Option layout for the root - reusing Option parser!
    let root_layout = parse_option_layout(db, root_field)?;
    
    Ok(MapDef {
        key_type,
        value_type,
        variant: MapVariant::BTreeMap {
            length_offset,
            root_offset,
            root_layout,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parser_api_compiles() {
        // Test that the API design compiles and looks clean
        let _field_parser = field("buf");
        let _offset_parser = field_offset("len");
        let _path_parser = field_path(&["buf", "inner", "ptr"]);
        let _generic_parser = generic("T");
        
        // The key insight: these parsers can be composed easily
        // and each does one specific thing well
    }
}