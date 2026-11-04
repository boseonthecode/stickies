use std::fs;

use uuid::Uuid;

use crate::error::StorageError;

use super::attachments;
use super::note::{AttachmentMeta, Note, NoteStore};
use super::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Clean,
    Deferred,
    Saved,
}

#[derive(Debug)]
pub struct NoteDraft {
    store: NoteStore,
    note: Note,
    persisted: bool,
    dirty: bool,
}

impl NoteDraft {
    pub fn new(store: NoteStore) -> Self {
        Self {
            store,
            note: Note::new(),
            persisted: false,
            dirty: false,
        }
    }

    pub fn existing(store: NoteStore, id: Uuid) -> Result<Self, StorageError> {
        let note = store.load(id)?;

        Ok(Self {
            store,
            note,
            persisted: true,
            dirty: false,
        })
    }

    pub fn id(&self) -> Uuid {
        self.note.meta.id
    }

    pub fn text(&self) -> &str {
        &self.note.body
    }

    pub fn attachments(&self) -> &[AttachmentMeta] {
        &self.note.meta.attachments
    }

    pub fn add_png(&mut self, bytes: &[u8]) -> Result<AttachmentMeta, StorageError> {
        let attachment =
            attachments::import_png_bytes(self.store.paths(), self.note.meta.id, bytes)?;
        self.note.meta.attachments.push(attachment.clone());
        self.dirty = true;
        Ok(attachment)
    }

    pub fn set_text(&mut self, text: String) {
        if self.note.body != text {
            self.note.body = text;
            self.dirty = true;
        }
    }

    pub fn is_persisted(&self) -> bool {
        self.persisted
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn has_content(&self) -> bool {
        !self.note.body.trim().is_empty() || !self.note.meta.attachments.is_empty()
    }

    pub fn save_if_needed(&mut self) -> Result<SaveOutcome, StorageError> {
        if !self.persisted && !self.has_content() {
            return Ok(SaveOutcome::Deferred);
        }

        if !self.dirty {
            return Ok(SaveOutcome::Clean);
        }

        self.store.save(&mut self.note)?;
        self.persisted = true;
        self.dirty = false;

        Ok(SaveOutcome::Saved)
    }

    pub fn finish(&mut self) -> Result<SaveOutcome, StorageError> {
        let outcome = self.save_if_needed()?;

        if outcome == SaveOutcome::Deferred {
            self.discard_if_empty()?;
        }

        Ok(outcome)
    }

    pub fn discard_if_empty(&mut self) -> Result<bool, StorageError> {
        if self.persisted || self.has_content() {
            return Ok(false);
        }

        let dir = paths::note_dir(self.store.paths(), self.note.meta.id);

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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StorageError::Read {
                    path: dir.clone(),
                    source,
                });
            }
        }

        self.dirty = false;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::config::DataPaths;

    use super::*;

    fn store() -> (TempDir, NoteStore) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let paths = DataPaths::with_root(temp.path().join("stickies"));
        let store = NoteStore::new(paths);
        (temp, store)
    }

    #[test]
    fn empty_new_draft_is_not_written() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());

        assert_eq!(
            draft.save_if_needed().expect("save attempt"),
            SaveOutcome::Deferred
        );
        assert!(!draft.is_persisted());
        assert!(store.list_ids().expect("notes listed").is_empty());
    }

    #[test]
    fn whitespace_only_new_draft_is_not_written() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        draft.set_text(" \n\t ".to_owned());

        assert_eq!(
            draft.save_if_needed().expect("save attempt"),
            SaveOutcome::Deferred
        );
        assert!(store.list_ids().expect("notes listed").is_empty());
    }

    #[test]
    fn new_draft_writes_after_real_content() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        draft.set_text("# A real note".to_owned());

        assert_eq!(
            draft.save_if_needed().expect("note saved"),
            SaveOutcome::Saved
        );
        assert!(draft.is_persisted());
        assert!(!draft.is_dirty());
        assert_eq!(
            draft.save_if_needed().expect("clean save"),
            SaveOutcome::Clean
        );

        let loaded = store.load(draft.id()).expect("note loaded");
        assert_eq!(loaded.body, "# A real note");
    }

    #[test]
    fn existing_note_is_clean_until_edited() {
        let (_temp, store) = store();
        let mut note = store.create().expect("note created");
        note.body = "original".to_owned();
        store.save(&mut note).expect("note saved");
        let mut draft = NoteDraft::existing(store.clone(), note.meta.id).expect("draft loaded");

        assert_eq!(
            draft.save_if_needed().expect("clean save"),
            SaveOutcome::Clean
        );

        draft.set_text("original".to_owned());
        assert!(!draft.is_dirty());

        draft.set_text("changed".to_owned());
        assert!(draft.is_dirty());
        assert_eq!(
            draft.save_if_needed().expect("note saved"),
            SaveOutcome::Saved
        );
        assert_eq!(
            store.load(note.meta.id).expect("note loaded").body,
            "changed"
        );
    }

    #[test]
    fn image_attachment_is_saved_without_body_text() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        let bytes = [0x89, b'P', b'N', b'G', 9, 8, 7];
        let attachment = draft.add_png(&bytes).expect("attachment added");

        assert!(draft.has_content());
        assert_eq!(draft.finish().expect("draft finished"), SaveOutcome::Saved);

        let loaded = store.load(draft.id()).expect("note loaded");
        assert!(loaded.body.is_empty());
        assert_eq!(loaded.meta.attachments, vec![attachment.clone()]);
        assert!(
            paths::attachments_dir(store.paths(), draft.id())
                .join(&attachment.stored_name)
                .exists()
        );

        let reopened = NoteDraft::existing(store, draft.id()).expect("draft reopened");
        assert_eq!(reopened.attachments(), &[attachment]);
    }

    #[test]
    fn existing_note_can_be_saved_empty() {
        let (_temp, store) = store();
        let mut note = store.create().expect("note created");
        note.body = "text".to_owned();
        store.save(&mut note).expect("note saved");
        let mut draft = NoteDraft::existing(store.clone(), note.meta.id).expect("draft loaded");
        draft.set_text(String::new());

        assert_eq!(
            draft.save_if_needed().expect("note saved"),
            SaveOutcome::Saved
        );
        assert!(
            store
                .load(note.meta.id)
                .expect("note loaded")
                .body
                .is_empty()
        );
    }

    #[test]
    fn finishing_an_empty_new_draft_removes_partial_state() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        let dir = paths::note_dir(store.paths(), draft.id());
        fs::create_dir_all(&dir).expect("partial directory");
        fs::write(dir.join("partial"), b"leftover").expect("partial file");

        assert_eq!(
            draft.finish().expect("draft finished"),
            SaveOutcome::Deferred
        );
        assert!(!dir.exists());
        assert!(store.list_ids().expect("notes listed").is_empty());
    }

    #[test]
    fn finishing_never_discards_a_persisted_note() {
        let (_temp, store) = store();
        let note = store.create().expect("note created");
        let dir = paths::note_dir(store.paths(), note.meta.id);
        let mut draft = NoteDraft::existing(store, note.meta.id).expect("draft loaded");

        assert_eq!(draft.finish().expect("draft finished"), SaveOutcome::Clean);
        assert!(dir.exists());
    }

    #[test]
    fn failed_save_keeps_the_draft_dirty() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let root = temp.path().join("stickies");
        fs::create_dir_all(&root).expect("root directory");
        fs::write(root.join("notes"), b"blocked").expect("blocking file");
        let store = NoteStore::new(DataPaths::with_root(root));
        let mut draft = NoteDraft::new(store);
        draft.set_text("will fail".to_owned());

        assert!(draft.save_if_needed().is_err());
        assert!(draft.is_dirty());
        assert!(!draft.is_persisted());
    }
}
