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

use std::path::PathBuf;

use anyhow::Result;
use rudy_dwarf::{Binary, DwarfDb, file::File};

#[salsa::db]
pub trait Db: salsa::Database + DwarfDb {}

#[salsa::db]
#[derive(Clone)]
pub struct DebugDatabaseImpl {
    storage: salsa::Storage<Self>,
    source_map: Vec<(PathBuf, PathBuf)>,
}

pub struct DebugDbRef {
    handle: salsa::StorageHandle<DebugDatabaseImpl>,
    source_map: Vec<(PathBuf, PathBuf)>,
}

impl DebugDbRef {
    pub fn get_db(self) -> DebugDatabaseImpl {
        DebugDatabaseImpl {
            storage: self.handle.into_storage(),
            source_map: self.source_map,
        }
    }
}

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
            source_map: Default::default(),
        }
    }

    /// Creates a new debug database instance with salsa event logging.
    ///
    /// The event callback will be called for all salsa events, allowing you to monitor
    /// query execution, cache hits/misses, and dependency tracking.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rudy_db::DebugDb;
    ///
    /// let db = DebugDb::new_with_events(Some(Box::new(|event| {
    ///     println!("Salsa event: {:?}", event);
    /// })));
    /// ```
    pub fn new_with_events(
        event_callback: Option<Box<dyn Fn(salsa::Event) + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            storage: salsa::Storage::new(event_callback),
            source_map: Default::default(),
        }
    }

    pub fn with_source_map(mut self, source_map: Vec<(PathBuf, PathBuf)>) -> Self {
        self.source_map = source_map;
        self
    }

    pub(crate) fn load_binary(&self, binary_file: PathBuf) -> Result<Binary> {
        let file = File::build(self, binary_file, None)?;
        Ok(Binary::new(self, file))
    }

    pub fn get_sync_ref(&self) -> DebugDbRef {
        DebugDbRef {
            handle: self.storage.clone().into_zalsa_handle(),
            source_map: self.source_map.clone(),
        }
    }
}

#[salsa::db]
impl salsa::Database for DebugDatabaseImpl {}

#[salsa::db]
impl DwarfDb for DebugDatabaseImpl {
    fn get_source_map(&self) -> &[(PathBuf, PathBuf)] {
        &self.source_map
    }
}

#[salsa::db]
impl Db for DebugDatabaseImpl {}
