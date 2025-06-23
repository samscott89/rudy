//! Resolution functionality for addresses, types, and variables

pub mod address;
pub mod functions;
pub mod types;
pub mod variables;

pub use address::*;
pub use functions::*;
pub use types::*;
pub use variables::*;

use std::{fmt, sync::Arc};

use crate::dwarf::die::DieAccessError;

#[derive(Debug, Clone)]
pub enum Error {
    Gimli(gimli::Error),
    Resolution(String),
    DieAccess(Arc<DieAccessError>),
    Custom(Arc<anyhow::Error>), // Io(Arc<std::io::Error>),
                                // ObjectParseError(object::read::Error),
                                // MemberFileNotFound(String),
}

unsafe impl salsa::Update for Error {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        unsafe { *old_pointer = new_value };
        true
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DieAccess(error) => write!(f, "Die access error: {error}"),
            Error::Gimli(error) => write!(f, "Gimli error: {error}"),
            Error::Resolution(error) => write!(f, "Resolution error: {error}"),
            Error::Custom(error) => write!(f, "{error}"),
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
            Error::DieAccess(error) => Some(error),
            Error::Gimli(error) => Some(error),
            Error::Resolution(_) => None,
            Error::Custom(error) => error.source(),
        }
    }
}

impl From<gimli::Error> for Error {
    fn from(err: gimli::Error) -> Self {
        Error::Gimli(err)
    }
}

impl From<String> for Error {
    fn from(err: String) -> Self {
        Error::Resolution(err)
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Error::Custom(Arc::new(err))
    }
}

impl From<DieAccessError> for Error {
    fn from(err: DieAccessError) -> Self {
        Error::DieAccess(Arc::new(err))
    }
}
