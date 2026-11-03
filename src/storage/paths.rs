use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::config::DataPaths;
use crate::error::StorageError;

pub fn note_dir(paths: &DataPaths, id: Uuid) -> PathBuf {
    paths.notes.join(id.to_string())
}

pub fn body_path(paths: &DataPaths, id: Uuid) -> PathBuf {
    note_dir(paths, id).join("note.md")
}

pub fn metadata_path(paths: &DataPaths, id: Uuid) -> PathBuf {
    note_dir(paths, id).join("meta.json")
}

pub fn attachments_dir(paths: &DataPaths, id: Uuid) -> PathBuf {
    note_dir(paths, id).join("attachments")
}

pub fn create_private_dir(path: &Path) -> Result<(), StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(StorageError::Symlink {
                path: path.to_path_buf(),
            });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(StorageError::NotDirectory {
                path: path.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(|source| StorageError::CreateDirectory {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        Err(source) => {
            return Err(StorageError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
        StorageError::SetPermissions {
            path: path.to_path_buf(),
            source,
        }
    })
}

pub fn cleanup_temp_files(dir: &Path) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".atomic-") && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}
