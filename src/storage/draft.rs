use std::fs;
use std::io;

use uuid::Uuid;

use crate::error::StorageError;

use super::attachments::{self, AttachmentEntry, FileCandidate, FileImportReport};
use super::note::{AttachmentMeta, Note, NoteStore};
use super::paths;
use super::tags;

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
        let mut note = store.load(id)?;
        note.meta.tags = tags::normalize_tags(&note.meta.tags);

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

    pub fn tags(&self) -> &[String] {
        &self.note.meta.tags
    }

    pub fn pinned(&self) -> bool {
        self.note.meta.pinned
    }

    pub fn add_tags(&mut self, tags: &[String]) -> bool {
        if tags::merge_tags(&mut self.note.meta.tags, tags) {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn remove_tag(&mut self, tag: &str) -> bool {
        if tags::remove_tag(&mut self.note.meta.tags, tag) {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn set_pinned(&mut self, pinned: bool) -> bool {
        if self.note.meta.pinned == pinned {
            return false;
        }

        self.note.meta.pinned = pinned;
        self.dirty = true;
        true
    }

    pub fn add_png(&mut self, bytes: &[u8]) -> Result<AttachmentMeta, StorageError> {
        let attachment =
            attachments::import_png_bytes(self.store.paths(), self.note.meta.id, bytes)?;
        self.add_attachment(attachment.clone());
        Ok(attachment)
    }

    pub fn add_attachment(&mut self, attachment: AttachmentMeta) {
        self.note.meta.attachments.push(attachment);
        self.dirty = true;
    }

    pub fn add_files(&mut self, files: &[FileCandidate]) -> Result<FileImportReport, StorageError> {
        let report = attachments::import_files(self.store.paths(), self.note.meta.id, files)?;

        if !report.imported.is_empty() {
            self.note
                .meta
                .attachments
                .extend(report.imported.iter().cloned());
            self.dirty = true;
        }

        Ok(report)
    }

    pub fn attachment_entries(&self) -> Vec<AttachmentEntry> {
        self.note
            .meta
            .attachments
            .iter()
            .map(|attachment| {
                attachments::attachment_entry(self.store.paths(), self.note.meta.id, attachment)
            })
            .collect()
    }

    pub fn remove_attachment(&mut self, attachment_id: Uuid) -> Result<bool, StorageError> {
        let Some(index) = self
            .note
            .meta
            .attachments
            .iter()
            .position(|attachment| attachment.id == attachment_id)
        else {
            return Ok(false);
        };

        let attachment = &self.note.meta.attachments[index];
        if let Some(path) = paths::attachment_path(
            self.store.paths(),
            self.note.meta.id,
            &attachment.stored_name,
        ) {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(StorageError::Symlink { path });
                }
                Ok(metadata) if !metadata.is_file() => {
                    return Err(StorageError::Remove {
                        path: path.clone(),
                        source: io::Error::new(
                            io::ErrorKind::InvalidData,
                            "attachment is not a regular file",
                        ),
                    });
                }
                Ok(_) => {
                    fs::remove_file(&path).map_err(|source| StorageError::Remove {
                        path: path.clone(),
                        source,
                    })?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(StorageError::Read { path, source });
                }
            }
        }

        self.note.meta.attachments.remove(index);
        self.dirty = true;
        Ok(true)
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
        !self.note.body.trim().is_empty()
            || !self.note.meta.attachments.is_empty()
            || !self.note.meta.tags.is_empty()
            || self.note.meta.pinned
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
    use crate::storage::note::AttachmentKind;

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
    fn tags_and_pinning_materialize_a_new_draft() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());

        assert!(draft.add_tags(&["#Work".to_owned(), "idea".to_owned()]));
        assert!(draft.set_pinned(true));
        assert!(!draft.add_tags(&["work".to_owned()]));

        assert_eq!(
            draft.save_if_needed().expect("note saved"),
            SaveOutcome::Saved
        );

        let loaded = store.load(draft.id()).expect("note loaded");
        assert_eq!(loaded.meta.tags, vec!["Work", "idea"]);
        assert!(loaded.meta.pinned);
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
    fn file_attachments_are_saved_without_body_text() {
        let (temp, store) = store();
        let source = temp.path().join("report.txt");
        fs::write(&source, b"report").expect("source file");
        let candidate = FileCandidate {
            path: source,
            original_name: "report.txt".to_owned(),
            size_bytes: 6,
        };
        let mut draft = NoteDraft::new(store.clone());

        let report = draft.add_files(&[candidate]).expect("files added");

        assert_eq!(report.imported.len(), 1);
        assert!(report.rejected.is_empty());
        assert!(draft.has_content());
        assert_eq!(draft.finish().expect("draft finished"), SaveOutcome::Saved);

        let loaded = store.load(draft.id()).expect("note loaded");
        assert!(loaded.body.is_empty());
        assert_eq!(loaded.meta.attachments, report.imported);
        assert_eq!(loaded.meta.attachments[0].original_name, "report.txt");
        assert_eq!(loaded.meta.attachments[0].kind, AttachmentKind::File);
    }

    #[test]
    fn attachment_entries_report_missing_files() {
        let (temp, store) = store();
        let source = temp.path().join("report.txt");
        fs::write(&source, b"report").expect("source file");
        let candidate = FileCandidate {
            path: source,
            original_name: "report.txt".to_owned(),
            size_bytes: 6,
        };
        let mut draft = NoteDraft::new(store);
        let report = draft.add_files(&[candidate]).expect("files added");

        let entries = draft.attachment_entries();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].missing);
        let stored_path = entries[0].path.clone().expect("attachment path");
        assert_eq!(entries[0].attachment, report.imported[0]);

        fs::remove_file(stored_path).expect("attachment removed from disk");

        let entries = draft.attachment_entries();
        assert!(entries[0].missing);
        assert!(
            draft
                .remove_attachment(entries[0].attachment.id)
                .expect("metadata removed")
        );
        assert!(draft.attachments().is_empty());
    }

    #[test]
    fn removes_attachment_file_and_metadata() {
        let (_temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        let attachment = draft
            .add_png(&[0x89, b'P', b'N', b'G'])
            .expect("attachment added");
        let path = paths::attachments_dir(store.paths(), draft.id()).join(&attachment.stored_name);
        assert!(path.exists());

        assert!(
            draft
                .remove_attachment(attachment.id)
                .expect("attachment removed")
        );

        assert!(!path.exists());
        assert!(draft.attachments().is_empty());
        assert!(draft.is_dirty());
    }

    #[test]
    fn refuses_to_remove_a_symlinked_attachment() {
        use std::os::unix::fs::symlink;

        let (temp, store) = store();
        let mut draft = NoteDraft::new(store.clone());
        let attachment = draft
            .add_png(&[0x89, b'P', b'N', b'G'])
            .expect("attachment added");
        let path = paths::attachments_dir(store.paths(), draft.id()).join(&attachment.stored_name);
        let target = temp.path().join("target.png");
        fs::write(&target, b"target").expect("target file");
        fs::remove_file(&path).expect("attachment file removed");
        symlink(&target, &path).expect("symlink created");

        assert!(matches!(
            draft.remove_attachment(attachment.id),
            Err(StorageError::Symlink { .. })
        ));
        assert_eq!(draft.attachments().len(), 1);
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
        assert_eq!(draft.text(), "will fail");
    }
}
