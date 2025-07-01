use std::{collections::BTreeSet, path::PathBuf};

use gimli::Reader as _;

use crate::{
    die::{
        cu::is_rust_cu, file_entry_to_path, navigation::get_roots, utils::pretty_print_die_entry,
    },
    DebugFile, DwarfDb, SourceFile,
};

/// Build an index of all types + functions (using fully qualified names
/// that can be extracted from demangled symbols) to their
/// corresponding DIE entry in the DWARF information.
#[salsa::tracked(returns(ref))]
pub fn index_debug_file_sources<'db>(
    db: &'db dyn DwarfDb,
    debug_file: DebugFile,
) -> (BTreeSet<PathBuf>, BTreeSet<SourceFile<'db>>) {
    let mut compile_dirs = BTreeSet::new();
    let mut sources = BTreeSet::new();

    let roots = get_roots(db, debug_file);
    for (_unit_offset, unit_ref) in &roots {
        let mut entries = unit_ref.entries();
        let Some((_, root)) = entries.next_dfs().ok().flatten() else {
            continue;
        };
        if is_rust_cu(root, unit_ref) {
            // get the compile directory
            if let Some(compile_dir) = &unit_ref.comp_dir {
                match compile_dir.to_string() {
                    Ok(compile_dir) => {
                        // if the compile directory is empty, we can skip it
                        if compile_dir.is_empty() {
                            tracing::debug!(
                                "Skipping empty compile directory for unit: {}",
                                pretty_print_die_entry(root, unit_ref)
                            );
                        } else {
                            compile_dirs.insert(PathBuf::from(compile_dir.to_string()));
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to convert compile directory to string: {e}");
                    }
                }
            }

            // get all referenced files
            let files = unit_ref
                .line_program
                .as_ref()
                .map(|lp| {
                    lp.header()
                        .file_names()
                        .iter()
                        .flat_map(|f| {
                            file_entry_to_path(db, f, unit_ref)
                                .map(|path| SourceFile::new(db, path))
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            sources.extend(files);
        }
    }

    (compile_dirs, sources)
}
