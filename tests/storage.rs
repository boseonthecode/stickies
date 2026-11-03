use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use stickies::config::DataPaths;
use stickies::error::StorageError;
use stickies::storage::note::{AttachmentKind, AttachmentMeta, META_SCHEMA_VERSION, NoteStore};
use stickies::storage::paths;
use tempfile::TempDir;
use uuid::Uuid;

fn test_store() -> (TempDir, NoteStore) {
    let temp = tempfile::tempdir().expect("temporary directory");
    let paths = DataPaths::with_root(temp.path().join("stickies"));
    let store = NoteStore::new(paths);
    (temp, store)
}

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path)
        .expect("path metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn round_trips_all_note_fields() {
    let (_temp, store) = test_store();
    let mut note = store.create().expect("note created");
    note.body = "# Heading\n\nSome body text.".to_owned();
    note.meta.tags = vec!["work".to_owned(), "idea".to_owned()];
    note.meta.pinned = true;
    note.meta.attachments.push(AttachmentMeta {
        id: Uuid::now_v7(),
        stored_name: "stored.png".to_owned(),
        original_name: "screenshot.png".to_owned(),
        mime: "image/png".to_owned(),
        size_bytes: 1234,
        kind: AttachmentKind::Image,
        created_at: note.meta.created_at,
    });
    store.save(&mut note).expect("note saved");

    let loaded = store.load(note.meta.id).expect("note loaded");

    assert_eq!(loaded.body, note.body);
    assert_eq!(loaded.meta.tags, note.meta.tags);
    assert!(loaded.meta.pinned);
    assert_eq!(loaded.meta.attachments, note.meta.attachments);
    assert_eq!(loaded.meta.schema_version, META_SCHEMA_VERSION);
    assert_eq!(loaded.meta.id, note.meta.id);
    assert_eq!(loaded.meta.created_at, note.meta.created_at);
}

#[test]
fn creates_distinct_sortable_note_ids() {
    let (_temp, store) = test_store();

    let first = store.create().expect("first note");
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = store.create().expect("second note");

    assert_ne!(first.meta.id, second.meta.id);
    assert_eq!(first.meta.id.get_version_num(), 7);
    assert!(first.meta.id < second.meta.id);
}

#[test]
fn loads_a_note_with_an_empty_body() {
    let (_temp, store) = test_store();
    let note = store.create().expect("note created");

    let loaded = store.load(note.meta.id).expect("note loaded");

    assert!(loaded.body.is_empty());
}

#[test]
fn missing_note_returns_not_found() {
    let (_temp, store) = test_store();

    let result = store.load(Uuid::now_v7());

    assert!(matches!(result, Err(StorageError::NotFound { .. })));
}

#[test]
fn lists_only_valid_note_directories() {
    let (_temp, store) = test_store();
    let first = store.create().expect("first note");
    let second = store.create().expect("second note");

    fs::create_dir(store.paths().notes.join("not-a-uuid")).expect("unrelated directory");
    fs::write(store.paths().notes.join("stray-file"), b"ignore").expect("stray file");

    let ids = store.list_ids().expect("notes listed");

    assert_eq!(ids, vec![first.meta.id, second.meta.id]);
}

#[test]
fn recovers_body_when_metadata_is_corrupt() {
    let (_temp, store) = test_store();
    let mut note = store.create().expect("note created");
    note.body = "body survives".to_owned();
    store.save(&mut note).expect("note saved");

    fs::write(
        paths::metadata_path(store.paths(), note.meta.id),
        b"{ this is not json",
    )
    .expect("corrupt metadata written");

    let loaded = store.load(note.meta.id).expect("note recovered");

    assert_eq!(loaded.body, "body survives");
    assert_eq!(loaded.meta.id, note.meta.id);
    assert!(loaded.meta.tags.is_empty());
    assert!(!loaded.meta.pinned);
}

#[test]
fn recovers_when_metadata_is_missing() {
    let (_temp, store) = test_store();
    let mut note = store.create().expect("note created");
    note.body = "body without metadata".to_owned();
    store.save(&mut note).expect("note saved");

    fs::remove_file(paths::metadata_path(store.paths(), note.meta.id)).expect("metadata removed");

    let loaded = store.load(note.meta.id).expect("note recovered");

    assert_eq!(loaded.body, "body without metadata");
    assert_eq!(loaded.meta.id, note.meta.id);
}

#[test]
fn missing_body_loads_as_empty() {
    let (_temp, store) = test_store();
    let note = store.create().expect("note created");

    fs::remove_file(paths::body_path(store.paths(), note.meta.id)).expect("body removed");

    let loaded = store.load(note.meta.id).expect("note loaded");

    assert!(loaded.body.is_empty());
    assert_eq!(loaded.meta.id, note.meta.id);
}

#[test]
fn overwrites_atomically_without_leaving_temp_files() {
    let (_temp, store) = test_store();
    let mut note = store.create().expect("note created");
    note.body = "first version".to_owned();
    store.save(&mut note).expect("first save");
    note.body = "second version".to_owned();
    store.save(&mut note).expect("second save");

    let body =
        fs::read_to_string(paths::body_path(store.paths(), note.meta.id)).expect("body read");
    assert_eq!(body, "second version");

    let dir = paths::note_dir(store.paths(), note.meta.id);
    let leftovers: Vec<_> = fs::read_dir(dir)
        .expect("note dir")
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(".atomic-") && name.ends_with(".tmp")
        })
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn cleans_stale_atomic_temp_files() {
    let (_temp, store) = test_store();
    let note = store.create().expect("note created");
    let stale = paths::note_dir(store.paths(), note.meta.id).join(".atomic-stale.tmp");
    fs::write(&stale, b"partial").expect("stale temp written");

    store.load(note.meta.id).expect("note loaded");

    assert!(!stale.exists());
}

#[test]
fn rejects_symlinked_note_directories() {
    let (temp, store) = test_store();
    let id = Uuid::now_v7();
    store.paths().ensure_dirs().expect("data directories");
    let external = temp.path().join("external");
    fs::create_dir(&external).expect("external directory");
    symlink(&external, store.paths().notes.join(id.to_string())).expect("symlink created");

    assert!(
        store.list_ids().expect("notes listed").is_empty(),
        "symlinked note directories must be skipped"
    );
    assert!(matches!(store.load(id), Err(StorageError::Symlink { .. })));
}

#[test]
fn rejects_unsupported_metadata_versions() {
    let (_temp, store) = test_store();
    let note = store.create().expect("note created");
    let path = paths::metadata_path(store.paths(), note.meta.id);
    let metadata = fs::read_to_string(&path).expect("metadata read");
    let metadata = metadata.replace(
        &format!("\"schema_version\": {META_SCHEMA_VERSION}"),
        "\"schema_version\": 99",
    );
    fs::write(&path, metadata).expect("future metadata written");

    assert!(matches!(
        store.load(note.meta.id),
        Err(StorageError::UnsupportedSchema { .. })
    ));
}

#[test]
fn writes_private_directories_and_files() {
    let (_temp, store) = test_store();
    let note = store.create().expect("note created");

    assert_eq!(mode(&paths::note_dir(store.paths(), note.meta.id)), 0o700);
    assert_eq!(mode(&paths::body_path(store.paths(), note.meta.id)), 0o600);
    assert_eq!(
        mode(&paths::metadata_path(store.paths(), note.meta.id)),
        0o600
    );
}
