use std::fmt;

use anyhow::Result;
use memmap2::Mmap;
use object::Object;
use object::read::archive::ArchiveFile;

use crate::database::Db;
use crate::dwarf::Dwarf;

#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum FilePath {
    /// A file path that is not part of an archive
    Path(String),
    /// A file path that is part of an archive, with the member name
    ArchiveMember { path: String, member: String },
}

impl fmt::Display for FilePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilePath::Path(path) => write!(f, "{}", path),
            FilePath::ArchiveMember { path, member } => write!(f, "{}({})", path, member),
        }
    }
}

#[salsa::input]
pub struct Binary {
    pub file_path: FilePath,
}

impl Binary {
    pub fn file_id<'db>(&self, db: &'db dyn Db) -> FileId<'db> {
        let path = self.file_path(db).to_string();
        FileId::new(db, path.clone(), None, false)
    }
}

pub struct LoadedFile {
    filepath: FilePath,
    mapped_file: Mmap,
    pub object: object::File<'static>,
    pub dwarf: Dwarf,
}

impl LoadedFile {
    pub fn new(path: FilePath) -> Result<Self> {
        let (file, member) = match &path {
            FilePath::Path(path) => (path, None),
            FilePath::ArchiveMember { path, member } => (path, Some(member.as_str())),
        };
        let (mapped_file, object, dwarf) = try_load(file, member)?;
        Ok(LoadedFile {
            filepath: path,
            mapped_file,
            object,
            dwarf,
        })
    }

    pub fn has_debug_symbols(&self) -> bool {
        self.object.has_debug_symbols()
    }
}

impl fmt::Debug for LoadedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedFile")
            .field("filepath", &self.filepath.to_string())
            .field("size", &self.mapped_file.len())
            .field("has_debug_sections", &self.object.has_debug_symbols())
            .finish()
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

    pub fn filepath(&self, db: &'db dyn Db) -> FilePath {
        let path = self.path(db).clone();
        if let Some(member) = self.member(db) {
            FilePath::ArchiveMember {
                path,
                member: member.clone(),
            }
        } else {
            FilePath::Path(path)
        }
    }
}

#[salsa::interned]
pub struct SourceFile<'db> {
    #[return_ref]
    pub path: String,
}
