//! Type indexing functionality

use crate::{
    die::{utils::get_string_attr, Die},
    file::DebugFile,
    modules::{module_index, ModuleRange},
    DwarfDb, TypeName,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct TypeIndexEntry<'db> {
    pub die: Die<'db>,
}

/// Find the namespace path for a given DIE offset using range lookup
fn find_namespace_for_offset(ranges: &[ModuleRange], target_offset: usize) -> Vec<String> {
    // Find the most specific (deepest) namespace that contains this offset

    // first, find the first node that starts _after_ the target offset -- we'll search backwards
    // from this one
    let Ok(pos) = ranges.binary_search_by(|range| {
        if target_offset < range.start_offset {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        }
    }) else {
        tracing::debug!("No namespace found for offset {target_offset:#x}");
        return Vec::new();
    };

    let Some(path) = ranges[..pos]
        .iter()
        .rev()
        .find(|range| target_offset >= range.start_offset && target_offset < range.end_offset)
    else {
        tracing::debug!("No namespace found for offset {target_offset:#x}");
        return Vec::new();
    };

    path.module_path.clone()
}

/// Get typename by lazily resolving namespace context (avoids full indexing)
/// This is salsa-cached for performance
#[salsa::tracked(returns(ref))]
pub fn get_die_typename<'db>(db: &'db dyn DwarfDb, die: Die<'db>) -> Option<TypeName> {
    die.with_entry_and_unit(db, |target_entry, unit_ref| {
        // Get the name of the target DIE
        let name = get_string_attr(target_entry, gimli::DW_AT_name, unit_ref)
            .ok()
            .flatten()?;

        // Get the namespace ranges for this debug file (cached)
        let module_index = module_index(db, die.file(db));

        // Find the module path for this DIE using range lookup
        let module_path =
            find_namespace_for_offset(module_index.by_offset(db), target_entry.offset().0);

        tracing::debug!(
            "Found module path: {:?} for DIE: {} at offset {:#x}",
            module_path,
            die.print(db),
            target_entry.offset().0,
        );

        // Parse the typename with the module path
        TypeName::parse(&module_path, &name).ok()
    })
    .ok()
    .flatten()
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
    let mut modules = module_index.by_name(db);
    let mut found_module = vec![];

    // tracing::info!("")

    for segment in &type_name.module.segments {
        if let Some(module) = modules.get(segment) {
            found_module = module.entries.clone();
            modules = &module.modules;
            tracing::info!(
                "Found module segment {segment} in debug file {} {:#?}",
                debug_file.name(db),
                modules.keys().collect::<Vec<_>>()
            );
        } else {
            tracing::info!(
                "Module segment {segment} not found in debug file {}",
                debug_file.name(db)
            );
        }
    }

    if found_module.is_empty() {
        tracing::warn!(
            "No module found for type {type_name:#?} in debug file {}\n\n{:#?}",
            debug_file.name(db),
            module_index.by_name(db).keys().collect::<Vec<_>>(),
        );
        return None;
    }

    tracing::info!(
        "Searching for type {type_name:#?} in modules: {:?}",
        found_module
    );

    // Now search for the type in the remaining modules
    for module in found_module {
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
