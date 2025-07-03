//! Type indexing functionality

use crate::{die::Die, file::DebugFile, modules::module_index, DwarfDb, TypeName};

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct TypeIndexEntry<'db> {
    pub die: Die<'db>,
}

/// Get typename by lazily resolving namespace context (avoids full indexing)
/// This is salsa-cached for performance
#[salsa::tracked(returns(ref))]
pub fn get_die_typename<'db>(db: &'db dyn DwarfDb, die: Die<'db>) -> Option<TypeName> {
    // Get the namespace ranges for this debug file (cached)
    let module_index = module_index(db, die.file(db));

    // Find the module path for this DIE using range lookup
    let Some(range) = module_index.find_by_offset(db, die.offset(db)) else {
        tracing::warn!(
            "No module range found for DIE at offset {:#x} in file {}",
            die.offset(db),
            die.file(db).name(db)
        );
        return None;
    };
    let name = die
        .name(db)
        .inspect_err(|e| {
            tracing::warn!("Failed to get name for DIE:: {e} {}", die.print(db));
        })
        .ok()?;
    // Parse the typename with the module path
    TypeName::parse(&range.module_path, &name).ok()
}

pub fn find_type_by_name<'db>(
    db: &'db dyn DwarfDb,
    debug_file: DebugFile,
    type_name: TypeName,
) -> Option<Die<'db>> {
    let module_index = module_index(db, debug_file);

    // let indexed = super::navigation::function_index(db, debug_file);
    // let type_name = TypeName::parse(type_name).ok()?;

    // Search through all debug files to find the type
    let Some(module) = module_index.find_by_path(db, &type_name.module.segments) else {
        tracing::warn!(
            "Module not found for type {type_name:#?} in debug file {}",
            debug_file.name(db)
        );
        return None;
    };

    tracing::trace!(
        "Searching for type {type_name:#?} in modules: {:?}",
        module.entries
    );

    // Now search for the type in the remaining modules
    for module in &module.entries {
        tracing::info!(
            "Searching in module: {} at location: {}",
            module.name(db).unwrap(),
            module.location(db)
        );
        // find the type name in the module
        for entry in module.children(db).unwrap_or_default() {
            if !matches!(
                entry.tag(db),
                gimli::DW_TAG_structure_type
                    | gimli::DW_TAG_enumeration_type
                    | gimli::DW_TAG_array_type
                    | gimli::DW_TAG_pointer_type
                    | gimli::DW_TAG_base_type
            ) {
                // Skip namespace entries
                continue;
            }
            if let Ok(name) = entry.name(db) {
                let Ok(parsed) = TypeName::parse(&type_name.module.segments, &name) else {
                    tracing::warn!("Failed to parse type name `{name}` in module {module:?}");
                    continue;
                };
                tracing::info!(
                    "Checking parsed types:\n{:?}\n  vs\n{:?}",
                    parsed.typedef,
                    type_name.typedef
                );
                if parsed.typedef.matching_type(&type_name.typedef) {
                    tracing::info!("Found type {type_name:#?}  {}", entry.location(db));
                    return Some(entry);
                }
            }
        }
    }

    None
}
