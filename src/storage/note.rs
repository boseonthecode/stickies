use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::DataPaths;
use crate::error::StorageError;

use super::{atomic, paths};

pub const META_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteMeta {
    pub schema_version: u32,
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub trashed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attachments: Vec<AttachmentMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentMeta {
    pub id: Uuid,
    pub stored_name: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: u64,
    pub kind: AttachmentKind,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub meta: NoteMeta,
    pub body: String,
}

impl Note {
    pub fn new() -> Self {
        let now = Utc::now();

        Self {
            meta: NoteMeta {
                schema_version: META_SCHEMA_VERSION,
                id: Uuid::now_v7(),
                created_at: now,
                modified_at: now,
                tags: Vec::new(),
                pinned: false,
                trashed: false,
                trashed_at: None,
                attachments: Vec::new(),
            },
            body: String::new(),
        }
    }
}

impl Default for Note {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct NoteStore {
    paths: DataPaths,
}

impl NoteStore {
    pub fn new(paths: DataPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &DataPaths {
        &self.paths
    }

    pub fn create(&self) -> Result<Note, StorageError> {
        let mut note = Note::new();
        self.save(&mut note)?;
        Ok(note)
    }

    pub fn save(&self, note: &mut Note) -> Result<(), StorageError> {
        self.paths.ensure_dirs()?;

        let dir = paths::note_dir(&self.paths, note.meta.id);
        paths::create_private_dir(&dir)?;

        if !note.meta.attachments.is_empty() {
            paths::create_private_dir(&paths::attachments_dir(&self.paths, note.meta.id))?;
        }

        paths::cleanup_temp_files(&dir);

        note.meta.schema_version = META_SCHEMA_VERSION;
        note.meta.modified_at = Utc::now();

        let metadata = serde_json::to_vec_pretty(&note.meta).map_err(|source| {
            StorageError::SerializeMetadata {
                id: note.meta.id,
                source,
            }
        })?;

        atomic::write(
            &paths::body_path(&self.paths, note.meta.id),
            note.body.as_bytes(),
        )?;
        atomic::write(&paths::metadata_path(&self.paths, note.meta.id), &metadata)?;

        Ok(())
    }

    pub fn load(&self, id: Uuid) -> Result<Note, StorageError> {
        let dir = paths::note_dir(&self.paths, id);
        let metadata = fs::symlink_metadata(&dir).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                StorageError::NotFound { id }
            } else {
                StorageError::Read {
                    path: dir.clone(),
                    source,
                }
            }
        })?;

        if metadata.file_type().is_symlink() {
            return Err(StorageError::Symlink { path: dir });
        }
        if !metadata.is_dir() {
            return Err(StorageError::NotDirectory { path: dir });
        }

        paths::cleanup_temp_files(&dir);

        let body_path = paths::body_path(&self.paths, id);
        let body = match read_optional(&body_path)? {
            Some(bytes) => String::from_utf8(bytes).map_err(|error| StorageError::InvalidBody {
                path: body_path,
                source: error.utf8_error(),
            })?,
            None => String::new(),
        };

        let meta = self.load_metadata(id)?;

        Ok(Note { meta, body })
    }

    pub fn list_ids(&self) -> Result<Vec<Uuid>, StorageError> {
        let entries = match fs::read_dir(&self.paths.notes) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StorageError::Read {
                    path: self.paths.notes.clone(),
                    source,
                });
            }
        };

        let mut ids = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|source| StorageError::Read {
                path: self.paths.notes.clone(),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| StorageError::Read {
                path: path.clone(),
                source,
            })?;

            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }

            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };

            if let Ok(id) = Uuid::parse_str(&name) {
                ids.push(id);
            }
        }

        ids.sort();
        Ok(ids)
    }

    fn load_metadata(&self, id: Uuid) -> Result<NoteMeta, StorageError> {
        let path = paths::metadata_path(&self.paths, id);
        let bytes = match read_optional(&path)? {
            Some(bytes) => bytes,
            None => return Ok(default_metadata(id)),
        };

        match serde_json::from_slice::<NoteMeta>(&bytes) {
            Ok(meta) => {
                if meta.schema_version > META_SCHEMA_VERSION {
                    return Err(StorageError::UnsupportedSchema {
                        id,
                        version: meta.schema_version,
                    });
                }
                if meta.id != id {
                    return Err(StorageError::IdMismatch {
                        expected: id,
                        found: meta.id,
                    });
                }
                Ok(meta)
            }
            Err(error) => {
                tracing::warn!(note_id = %id, %error, "recovering note with unreadable metadata");
                Ok(default_metadata(id))
            }
        }
    }
}

fn default_metadata(id: Uuid) -> NoteMeta {
    let now = Utc::now();

    NoteMeta {
        schema_version: META_SCHEMA_VERSION,
        id,
        created_at: now,
        modified_at: now,
        tags: Vec::new(),
        pinned: false,
        trashed: false,
        trashed_at: None,
        attachments: Vec::new(),
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StorageError::Symlink {
            path: path.to_path_buf(),
        }),
        Ok(metadata) if !metadata.is_file() => Err(StorageError::Read {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"),
        }),
        Ok(_) => fs::read(path)
            .map(Some)
            .map_err(|source| StorageError::Read {
                path: path.to_path_buf(),
                source,
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StorageError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}
