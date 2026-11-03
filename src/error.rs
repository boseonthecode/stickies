use std::io;
use std::path::PathBuf;

use thiserror::Error;

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
