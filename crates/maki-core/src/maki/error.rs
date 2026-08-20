use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    ReadDirectoryFailed(PathBuf),
    ReadNoteFailed(PathBuf),
    ReadProjectFileFailed(PathBuf),
    InvalidProjectFile(PathBuf, String),
    InvalidNotePath(PathBuf),
    RootNotFound(PathBuf),
    RootNotDirectory(PathBuf),
    NoteNotFound(PathBuf),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::RootNotFound(path) => {
                write!(f, "Root not found: {}", path.display())
            }
            Error::RootNotDirectory(path) => {
                write!(f, "Root not a directory: {}", path.display())
            }
            Error::ReadDirectoryFailed(path) => {
                write!(f, "Read directory failed: {}", path.display())
            }
            Error::InvalidNotePath(path) => {
                write!(f, "Invalid note path: {}", path.display())
            }
            Error::NoteNotFound(path) => {
                write!(f, "Note not found: {}", path.display(),)
            }
            Error::ReadNoteFailed(path) => {
                write!(f, "Read note failed: {}", path.display())
            }
            Error::ReadProjectFileFailed(path) => {
                write!(f, "Read project file failed: {}", path.display())
            }
            Error::InvalidProjectFile(path, message) => {
                write!(f, "Invalid project file {}: {}", path.display(), message)
            }
        }
    }
}
