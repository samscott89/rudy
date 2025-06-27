//! Salsa database for debugging information
//!
//! This module provides a salsa database for debugging information.
//!
//! The core idea is to make debugging information (today: just DWARF)
//! available via salsa queries.
//!
//! The main benefit this provides is to make queries lazy, memoized, and
//! incremental.
//!
//! ## Approach
//!
//! The structure of debugging information in DWARF means that it's realtively
//! cheap to look things up once you know where they are, but finding it
//! requires walking/parsing multiple files/sections. Furthermore, there is
//! information that cannot be eagerly loaded, such as the location of variables
//! in memory since it depends on the current state of the program.
//!
//! Given all of this, we take a multi-pass approach:
//!
//! 1. Up-front, walk all the files to construct indexes into the debugging information
//!    that makes it quick to find the relevant files/sections. e.g.
//!     - Symbol -> compilation unit + offset
//!     - Source file -> compilation unit
//!     - Address -> relevant compilation units/sections
//!
//! This indexing happens on startup/initial loading of the files and
//! only changes if the binary is recompiled (although we should be able
//! to memoize anything looked up from the individual files).
//!
//! 2. Lazily parse specific sections and memoize the results. This is only
//!    called whenever we need the information (e.g. when breaking on a line inside a function).
//!    But the results should be able to be memoized and ~never recomputed, even when stepping
//!    through a debug session
//! 3. Session-specific recomputed values. There are some things that we always need to recompute
//!    depending on the current session. E.g. when getting locations for variables when inside a
//!    function, or parsing stack frames. These will typically use a lot of cached/memoized
//!    intermediate results, but are unlikely to be themselves cached.
//!
//!
//! NOTE: today we don't actually have _any_ inputs. There is no incrementality since we're
//! assuming the debug information is static. However, in the future we may want incrementality
//! in via making the Binary file and all object files inputs -- this way if we recompile the
//! binary we can recompute which parts of the binary are the same and which are unchanged.

use std::fmt::Debug;

use anyhow::Result;
use salsa::Accumulator;

use crate::file::{Binary, DebugFile, File};

#[salsa::db]
pub trait Db: salsa::Database {
    fn report_info(&self, message: String) {
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Info,
        }
        .accumulate(self);
    }
    fn report_warning(&self, message: String) {
        tracing::warn!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Warning,
        }
        .accumulate(self);
    }
    fn report_critical(&self, message: String) {
        tracing::error!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Critical,
        }
        .accumulate(self);
    }
    fn report_error(&self, message: String) {
        tracing::warn!("{message}");
        Diagnostic {
            message,
            severity: DiagnosticSeverity::Error,
        }
        .accumulate(self);
    }

    fn upcast(&self) -> &dyn Db;
}

#[derive(Clone, Copy, Debug)]
enum DiagnosticSeverity {
    /// Errors that we never expect to see
    /// and imply an internal error.
    Critical,
    Error,
    Warning,
    Info,
}

#[salsa::accumulator]
pub struct Diagnostic {
    message: String,
    severity: DiagnosticSeverity,
}

#[salsa::db]
#[derive(Clone)]
pub struct DebugDatabaseImpl {
    storage: salsa::Storage<Self>,
}

pub struct DebugDbRef {
    handle: salsa::StorageHandle<DebugDatabaseImpl>,
}

impl DebugDbRef {
    pub fn get_db(self) -> DebugDatabaseImpl {
        DebugDatabaseImpl {
            storage: self.handle.into_storage(),
        }
    }
}

pub fn handle_diagnostics(diagnostics: &[&Diagnostic]) -> Result<()> {
    let mut err = None;
    for d in diagnostics {
        match d.severity {
            DiagnosticSeverity::Critical => {
                if err.is_some() {
                    tracing::error!("Critical error: {}", d.message);
                } else {
                    err = Some(anyhow::anyhow!("Critical error: {}", d.message));
                }
            }
            DiagnosticSeverity::Error => {
                if err.is_some() {
                    tracing::error!("Error: {}", d.message);
                } else {
                    err = Some(anyhow::anyhow!("Error: {}", d.message));
                }
            }
            DiagnosticSeverity::Warning => {
                tracing::warn!("Warning: {}", d.message);
            }
            DiagnosticSeverity::Info => {
                tracing::info!("Info: {}", d.message);
            }
        }
    }

    if let Some(e) = err { Err(e) } else { Ok(()) }
}

// #[salsa::tracked]
// fn initialize<'db>(db: &'db dyn Db) {
//     // Generate the index on startup to save time
//     let _ = index::build_index(db);
// }

impl Default for DebugDatabaseImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugDatabaseImpl {
    /// Creates a new debug database instance.
    ///
    /// The database manages the loading and caching of debug information from binary files.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rudy_db::DebugDb;
    ///
    /// let db = DebugDb::new();
    /// ```
    pub fn new() -> Self {
        Self {
            storage: salsa::Storage::default(),
        }
    }

    pub(crate) fn analyze_file(&self, binary_file: &str) -> Result<(Binary, Vec<DebugFile>)> {
        // TODO: we should do some deduplication here to see if we've already loaded
        // this file. We can do thy by checking file path, size, mtime, etc.
        let file = File::build(self, binary_file.to_string(), None)?;
        let bin = Binary::new(self, file);

        let index = crate::index::debug_index(self, bin);
        tracing::debug!("Index built: {index:#?}");

        // get a list of all debug files so that we can track them in case they change
        let debug_files = index.debug_files(self).values().copied().collect();

        Ok((bin, debug_files))
    }

    pub fn get_sync_ref(&self) -> DebugDbRef {
        DebugDbRef {
            handle: self.storage.clone().into_zalsa_handle(),
        }
    }
}

#[salsa::db]
impl salsa::Database for DebugDatabaseImpl {}

#[salsa::db]
impl Db for DebugDatabaseImpl {
    fn upcast(&self) -> &dyn Db {
        self
    }
}
