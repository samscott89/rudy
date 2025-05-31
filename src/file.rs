use std::fmt;
use std::sync::Arc;

use memmap2::Mmap;
use object::Object;
use object::read::archive::ArchiveFile;

use crate::database::Db;
use crate::dwarf::Dwarf;

#[salsa::input(debug)]
pub struct File {
    #[returns(ref)]
    pub path: String,
    #[returns(ref)]
    pub member_file: Option<String>,
    pub mtime: std::time::SystemTime,
    pub size: u64,
}

impl File {
    /// Builds the `File` input from a file path and an optional member file name.`
    pub fn build(db: &dyn Db, path: String, member_file: Option<String>) -> anyhow::Result<File> {
        let file = std::fs::File::open(&path)?;
        let metadata = file.metadata()?;
        let mtime = metadata.modified()?;
        let size = metadata.len();

        Ok(Self::new(db, path, member_file, mtime, size))
    }
}

#[salsa::input(debug)]
pub struct Binary {
    pub file: File,
}

#[salsa::tracked]
pub struct DebugFile<'db> {
    /// The underlying file/metadata for this debug file
    pub file: File,
    /// Whether this debug file is relocatable
    /// (i..e it is split from the main binary and can be loaded independently)
    pub relocatable: bool,
}

pub struct LoadedFile {
    filepath: String,
    file: File,
    mapped_file: Mmap,
    pub object: object::File<'static>,
    pub dwarf: Dwarf,
}

impl PartialEq for LoadedFile {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
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

#[salsa::tracked(returns(ref))]
pub fn load<'db>(db: &'db dyn Db, file: File) -> Result<LoadedFile, Error> {
    let path = file.path(db);
    let member = file.member_file(db);
    let file_handle = std::fs::File::open(&path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file_handle) }?;
    let mmap_static_ref = unsafe {
        // SAFETY: we hold onto the Mmap until the end of the program
        // and we ensure it lives long enough
        std::mem::transmute::<&[u8], &'static [u8]>(&*mmap)
    };
    let object = if let Some(member) = &member {
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
            return Err(Error::MemberFileNotFound(format!(
                "object file {member} not found in archive {path}"
            )));
        }
    } else {
        object::read::File::parse(mmap_static_ref)?
    };
    let dwarf = super::dwarf::load(&object)?;

    Ok(LoadedFile {
        filepath: if let Some(member) = member {
            format!("{path}({member}")
        } else {
            path.to_string()
        },
        file,
        mapped_file: mmap,
        object,
        dwarf,
    })
}

#[salsa::interned(debug)]
#[derive(PartialOrd, Ord)]
pub struct SourceFile<'db> {
    #[returns(ref)]
    pub path: String,
}

#[derive(Debug, Clone)]
pub enum Error {
    Gimli(gimli::Error),
    Io(Arc<std::io::Error>),
    ObjectParseError(object::read::Error),
    MemberFileNotFound(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Gimli(error) => write!(f, "Gimli error: {error}"),
            Error::Io(error) => write!(f, "IO Error: {error}",),
            Error::MemberFileNotFound(e) => write!(f, "Member file not found: {e}"),
            Error::ObjectParseError(error) => write!(f, "Object parse error: {error}"),
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, _other: &Self) -> bool {
        // we'll consider _all_ errors equal for now
        // since we only really care about if it was an error or not
        true
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Gimli(error) => Some(error),
            Error::Io(error) => Some(error.as_ref()),
            Error::ObjectParseError(error) => Some(error),
            Error::MemberFileNotFound(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(Arc::new(err))
    }
}

impl From<object::read::Error> for Error {
    fn from(err: object::read::Error) -> Self {
        Error::ObjectParseError(err)
    }
}

impl From<gimli::Error> for Error {
    fn from(err: gimli::Error) -> Self {
        Error::Gimli(err)
    }
}
