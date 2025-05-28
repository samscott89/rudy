use core::fmt;
use std::sync::Arc;

use anyhow::Result;
use memmap2::Mmap;
use object::read::archive::ArchiveFile;

use super::{Db, dwarf::Dwarf};

/// Files loaded and parsed as objects
///
/// NOTE: we don't _track_ these because currently we assume that these
/// are not modified after they are loaded.
///
/// In the future, we may want to have a persistent debug session, allowing
/// for recompiling without recomputing the whole debug session.
/// But for now, we just load the files once and keep them in memory.
#[derive(Clone, Debug)]
pub struct File {
    path: String,
    member: Option<String>,
    loaded: Arc<LoadedObjectFile>,
}

impl File {
    pub fn new(path: &str, member: Option<&str>) -> Self {
        Self {
            path: path.to_string(),
            member: member.map(|s| s.to_string()),
            loaded: Arc::new(LoadedObjectFile::new(path, member)),
        }
    }

    pub fn object(&self) -> Option<&object::File<'static>> {
        match &*self.loaded {
            LoadedObjectFile::Loaded { object, .. } => Some(object),
            LoadedObjectFile::Errored { .. } => None,
        }
    }

    pub fn dwarf(&self) -> Option<&Dwarf> {
        match &*self.loaded {
            LoadedObjectFile::Loaded { dwarf, .. } => Some(dwarf),
            LoadedObjectFile::Errored { .. } => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match &*self.loaded {
            LoadedObjectFile::Loaded { .. } => None,
            LoadedObjectFile::Errored { message } => Some(message),
        }
    }

    pub fn file_id<'db>(&self, db: &'db dyn Db) -> FileId<'db> {
        let relocatable = self.path.ends_with(".o") || self.path.ends_with(".rlib");
        FileId::new(db, self.path.clone(), self.member.clone(), relocatable)
    }
}

impl std::hash::Hash for File {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl PartialEq for File {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for File {}

enum LoadedObjectFile {
    Loaded {
        #[allow(dead_code)]
        mapped_file: Mmap,
        object: object::File<'static>,
        dwarf: Dwarf,
    },
    Errored {
        message: String,
    },
}

impl fmt::Debug for LoadedObjectFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadedObjectFile::Loaded { .. } => write!(f, "Loaded"),
            LoadedObjectFile::Errored { message } => write!(f, "Errored: {message}"),
        }
    }
}

impl LoadedObjectFile {
    fn new(path: &str, member: Option<&str>) -> Self {
        match try_load(path, member) {
            Ok((mmap, object, dwarf)) => LoadedObjectFile::Loaded {
                mapped_file: mmap,
                object,
                dwarf,
            },
            Err(e) => LoadedObjectFile::Errored {
                message: e.to_string(),
            },
        }
    }
}

fn try_load(path: &str, member: Option<&str>) -> Result<(Mmap, object::File<'static>, Dwarf)> {
    let file = std::fs::File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file) }?;
    let mmap_static_ref = unsafe {
        // SAFETY: we hold onto the Mmap until the end of the program
        // and we ensure it lives long enough
        std::mem::transmute::<&[u8], &'static [u8]>(&*mmap)
    };
    let object = if let Some(member) = member {
        // we need to extract the object file from the archive
        let archive = ArchiveFile::parse(mmap_static_ref)?;
        if let Some(file) = archive.members().find_map(|file| {
            let Ok(file) = file else {
                return None;
            };
            if file.name() == member.as_bytes() {
                Some(file)
            } else {
                None
            }
        }) {
            // parse the object file from the slice of data
            // in the archive
            object::File::parse(file.data(mmap_static_ref)?)?
        } else {
            return Err(anyhow::anyhow!(
                "object file {member} not found in archive {path}"
            ));
        }
    } else {
        object::read::File::parse(mmap_static_ref)?
    };
    let dwarf = super::dwarf::load(&object)?;

    Ok((mmap, object, dwarf))
}

#[salsa::tracked]
pub struct DebugFile<'db> {
    id: FileId<'db>,
    #[return_ref]
    pub file: File,
}

#[salsa::tracked]
pub fn load_relocatable_file<'db>(db: &'db dyn Db, file_id: FileId<'db>) -> Option<DebugFile<'db>> {
    let path = file_id.path(db);
    let member = file_id.member(db);
    let file = File::new(path, member.as_deref());
    if let Some(e) = file.error() {
        tracing::error!(
            "Failed to load relocatable file: {e} - {}",
            std::backtrace::Backtrace::capture()
        );
        db.report_critical(format!("Failed to load relocatable file: {path}: {e}"));
        None
    } else {
        Some(DebugFile::new(db, file_id, file))
    }
}

#[salsa::interned]
pub struct FileId<'db> {
    /// File path
    #[return_ref]
    pub path: String,
    /// (Optional) when the file is a member of an archive
    /// this is the name of the member
    #[return_ref]
    pub member: Option<String>,
    pub relocatable: bool,
}

impl<'db> FileId<'db> {
    pub fn full_path(&self, db: &'db dyn Db) -> String {
        let path = self.path(db);
        if let Some(member) = self.member(db) {
            format!("{path}({member})")
        } else {
            path.clone()
        }
    }
}

#[salsa::interned]
pub struct SourceFile<'db> {
    #[return_ref]
    pub path: String,
}
