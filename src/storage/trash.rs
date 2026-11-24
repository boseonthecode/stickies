use std::fs;
use std::io;

use chrono::Utc;
use uuid::Uuid;

use crate::error::StorageError;

use super::note::{Note, NoteStore};
use super::paths;

pub fn trash(store: &NoteStore, id: Uuid) -> Result<Note, StorageError> {
    set_trashed(store, id, true)
}

pub fn restore(store: &NoteStore, id: Uuid) -> Result<Note, StorageError> {
    set_trashed(store, id, false)
}

pub fn purge(store: &NoteStore, id: Uuid) -> Result<(), StorageError> {
    let note = store.load(id)?;
    if !note.meta.trashed {
        return Err(StorageError::NotTrashed { id });
    }

    let dir = paths::note_dir(store.paths(), id);
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(StorageError::Symlink { path: dir });
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(StorageError::NotDirectory { path: dir });
        }
        Ok(_) => {
            fs::remove_dir_all(&dir).map_err(|source| StorageError::Remove {
                path: dir.clone(),
                source,
            })?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(StorageError::NotFound { id });
        }
        Err(source) => {
            return Err(StorageError::Read { path: dir, source });
        }
    }

    if let Some(index) = store.index()
        && let Err(error) = index.remove(id)
    {
        tracing::warn!(note_id = %id, %error, "failed to remove note from search index");
    }

    Ok(())
}

fn set_trashed(store: &NoteStore, id: Uuid, trashed: bool) -> Result<Note, StorageError> {
    let mut note = store.load(id)?;
    note.meta.trashed = trashed;
    note.meta.trashed_at = trashed.then(Utc::now);
    store.save(&mut note)?;
    Ok(note)
}
