use std::io;
use std::path::PathBuf;

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("neither XDG_DATA_HOME nor HOME is set")]
    MissingDataHome,
    #[error("failed to create directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to set permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("failed to create directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove {path}: {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to set permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize metadata for note {id}: {source}")]
    SerializeMetadata {
        id: Uuid,
        #[source]
        source: serde_json::Error,
    },
    #[error("note {id} does not exist")]
    NotFound { id: Uuid },
    #[error("note {id} must be moved to Trash before permanent deletion")]
    NotTrashed { id: Uuid },
    #[error("expected a directory at {path}")]
    NotDirectory { path: PathBuf },
    #[error("refusing to follow symlink at {path}")]
    Symlink { path: PathBuf },
    #[error("refusing to store an empty attachment")]
    EmptyAttachment,
    #[error("metadata id mismatch: expected {expected}, found {found}")]
    IdMismatch { expected: Uuid, found: Uuid },
    #[error("unsupported metadata schema version {version} for note {id}")]
    UnsupportedSchema { id: Uuid, version: u32 },
    #[error("note body is not valid UTF-8 at {path}: {source}")]
    InvalidBody {
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("index database error at {path}: {source}")]
    Database {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to reset index file {path}: {source}")]
    Reset {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to set permissions on index file {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Storage(#[from] StorageError),
}
