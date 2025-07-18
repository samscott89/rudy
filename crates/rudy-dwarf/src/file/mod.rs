use std::{
    fmt::{self, Debug},
    path::PathBuf,
    sync::Arc,
};

use memmap2::Mmap;
use object::{read::archive::ArchiveFile, Object};

pub(crate) mod index;
pub(crate) mod loader;

pub use index::index_debug_file_sources;
pub(crate) use loader::{Dwarf, DwarfReader, RawDie};

use crate::DwarfDb;

pub type Expression = gimli::Expression<DwarfReader>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, salsa::Update)]
pub struct SourceLocation {
    pub file: SourceFile,
    pub line: u64,
    pub column: Option<u64>,
}

impl SourceLocation {
    pub fn new(file: SourceFile, line: u64, column: Option<u64>) -> Self {
        Self { file, line, column }
    }
}

pub fn detect_cargo_root() -> Option<PathBuf> {
    fn cargo_detect_workspace() -> Option<PathBuf> {
        let output = std::process::Command::new(env!("CARGO"))
            .arg("locate-project")
            .arg("--workspace")
            .arg("--message-format=plain")
            .output()
            .ok()?
            .stdout;
        let cargo_path = PathBuf::from(std::str::from_utf8(&output).ok()?.trim());
        Some(cargo_path.parent()?.to_path_buf())
    }

    fn cwd_detect_workspace() -> Option<PathBuf> {
        let mut path = std::env::current_dir().ok()?;
        while !path.join("Cargo.toml").exists() {
            path = path.parent()?.to_path_buf();
        }
        Some(path)
    }

    // use cargo to detect workspace, falling back to a manual
    // detection if cargo is not available
    cargo_detect_workspace().or_else(cwd_detect_workspace)
}

#[salsa::input(debug)]
pub struct File {
    #[returns(ref)]
    pub path: PathBuf,
    #[returns(ref)]
    pub member_file: Option<String>,
    pub mtime: std::time::SystemTime,
    pub size: u64,
}

impl File {
    /// Builds the `File` input from a file path and an optional member file name.`
    pub fn build(
        db: &dyn DwarfDb,
        path: PathBuf,
        member_file: Option<String>,
    ) -> anyhow::Result<File> {
        // check if we the file needs to be relocated
        let path = db.remap_path(&path);

        let file = std::fs::File::open(&path).inspect_err(|_| {
            tracing::warn!("Failed to open file: {}:", path.display());
        })?;
        let metadata = file.metadata()?;
        let mtime = metadata.modified()?;
        let size = metadata.len();

        Ok(Self::new(db, path, member_file, mtime, size))
    }

    pub fn name(&self, db: &dyn salsa::Database) -> String {
        self.path(db).display().to_string()
    }
}

#[salsa::input(debug)]
pub struct Binary {
    pub file: File,
}

impl Binary {
    pub fn name(&self, db: &dyn DwarfDb) -> String {
        self.file(db).name(db)
    }
}

#[salsa::input(debug)]
#[derive(PartialOrd, Ord)]
pub struct DebugFile {
    /// The underlying file/metadata for this debug file
    pub file: File,
    /// Whether this debug file is relocatable
    /// (i..e it is split from the main binary and can be loaded independently)
    pub relocatable: bool,
}

impl DebugFile {
    pub fn name(&self, db: &dyn salsa::Database) -> String {
        let file = self.file(db);
        if let Some(member) = file.member_file(db) {
            format!("{}({})", file.name(db), member)
        } else {
            file.name(db)
        }
    }
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
pub fn load<'db>(db: &'db dyn DwarfDb, file: File) -> Result<LoadedFile, Error> {
    let path = file.path(db);
    let path = db.remap_path(path);
    let member = file.member_file(db);

    let file_handle = std::fs::File::open(path)
        .inspect_err(|_| tracing::warn!("Failed to open file: {}", file.name(db)))?;
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
                "object file {member} not found in archive {}",
                file.name(db)
            )));
        }
    } else {
        object::read::File::parse(mmap_static_ref)?
    };
    let dwarf = self::loader::load(&object)?;

    Ok(LoadedFile {
        filepath: if let Some(member) = member {
            format!("{}({member}", file.name(db))
        } else {
            file.name(db)
        },
        file,
        mapped_file: mmap,
        object,
        dwarf,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, salsa::Update)]
pub struct SourceFile {
    pub path: PathBuf,
}

impl SourceFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path_str(&self) -> std::borrow::Cow<str> {
        self.path.to_string_lossy()
    }

    pub fn is_external(&self) -> bool {
        // Check if the source file is external by checking if it is not in the same directory as the binary
        let current_dir = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        !self.path.starts_with(&current_dir)
    }
}

#[derive(Debug, Clone)]
pub enum Error {
    Gimli(gimli::Error),
    Io(Arc<std::io::Error>),
    ObjectParseFailure(object::read::Error),
    MemberFileNotFound(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Gimli(error) => write!(f, "Gimli error: {error}"),
            Error::Io(error) => write!(f, "IO Error: {error}",),
            Error::MemberFileNotFound(e) => write!(f, "Member file not found: {e}"),
            Error::ObjectParseFailure(error) => write!(f, "Object parse error: {error}"),
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
            Error::ObjectParseFailure(error) => Some(error),
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
        Error::ObjectParseFailure(err)
    }
}

impl From<gimli::Error> for Error {
    fn from(err: gimli::Error) -> Self {
        Error::Gimli(err)
    }
}
