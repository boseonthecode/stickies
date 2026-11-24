use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

use chrono::Utc;
use stickies::config::DataPaths;
use stickies::error::StorageError;
use stickies::storage::index::{SearchIndex, TrashFilter};
use stickies::storage::note::{AttachmentKind, AttachmentMeta, Note, NoteStore};
use stickies::storage::{paths, trash};
use tempfile::TempDir;
use uuid::Uuid;

fn test_paths() -> (TempDir, DataPaths) {
    let temp = tempfile::tempdir().expect("temporary directory");
    let paths = DataPaths::with_root(temp.path().join("stickies"));
    paths.ensure_dirs().expect("data directories");
    (temp, paths)
}

fn seed_note(paths: &DataPaths, body: &str, tags: &[&str]) -> Uuid {
    let store = NoteStore::new(paths.clone());
    let mut note = store.create().expect("note created");
    note.body = body.to_owned();
    note.meta.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
    store.save(&mut note).expect("note saved");
    note.meta.id
}

fn search(index: &SearchIndex, query: &str) -> Vec<Uuid> {
    index
        .search(query, TrashFilter::Exclude)
        .expect("search succeeds")
}

#[test]
fn rebuilds_from_note_files_on_first_open() {
    let (_temp, paths) = test_paths();
    let body_id = seed_note(&paths, "Project alpha kickoff notes", &[]);
    let tag_id = seed_note(&paths, "Unrelated text", &["work", "ideas"]);
    let attachment_id = seed_note(&paths, "Attachment owner", &[]);

    let store = NoteStore::new(paths.clone());
    let mut note = store.load(attachment_id).expect("note loaded");
    note.meta.attachments.push(AttachmentMeta {
        id: Uuid::now_v7(),
        stored_name: "stored.png".to_owned(),
        original_name: "architecture-diagram.png".to_owned(),
        mime: "image/png".to_owned(),
        size_bytes: 42,
        kind: AttachmentKind::Image,
        created_at: Utc::now(),
    });
    store.save(&mut note).expect("note saved");

    let index = SearchIndex::open(&paths).expect("index opened");

    assert_eq!(search(&index, "alpha"), vec![body_id]);
    assert_eq!(search(&index, "ALPHA"), vec![body_id]);
    assert_eq!(search(&index, "wor"), vec![tag_id]);
    assert_eq!(search(&index, "#work"), vec![tag_id]);
    assert_eq!(search(&index, "ideas"), vec![tag_id]);
    assert_eq!(search(&index, "architecture"), vec![attachment_id]);
    assert_eq!(search(&index, "diagram"), vec![attachment_id]);
    assert!(search(&index, "missing-term").is_empty());
}

#[test]
fn incremental_save_updates_search_results() {
    let (_temp, paths) = test_paths();
    let index = Rc::new(SearchIndex::open(&paths).expect("index opened"));
    let store = NoteStore::new(paths.clone()).with_index(Rc::clone(&index));

    let mut note = store.create().expect("note created");
    note.body = "first draft".to_owned();
    store.save(&mut note).expect("note saved");

    assert_eq!(search(&index, "first"), vec![note.meta.id]);

    note.body = "second version".to_owned();
    store.save(&mut note).expect("note saved");

    assert_eq!(search(&index, "second"), vec![note.meta.id]);
    assert!(search(&index, "first").is_empty());
}

#[test]
fn tag_removal_updates_search_results() {
    let (_temp, paths) = test_paths();
    let index = Rc::new(SearchIndex::open(&paths).expect("index opened"));
    let store = NoteStore::new(paths.clone()).with_index(Rc::clone(&index));
    let mut note = store.create().expect("note created");
    note.body = "tagged note".to_owned();
    note.meta.tags = vec!["work".to_owned(), "idea".to_owned()];
    store.save(&mut note).expect("note saved");
    let id = note.meta.id;

    assert_eq!(search(&index, "#work"), vec![id]);

    note.meta.tags.retain(|tag| tag != "work");
    store.save(&mut note).expect("note saved");

    assert!(search(&index, "#work").is_empty());
    assert_eq!(search(&index, "#idea"), vec![id]);
}

#[test]
fn empty_query_lists_pinned_notes_first() {
    let (_temp, paths) = test_paths();
    let index = SearchIndex::open(&paths).expect("index opened");
    let store = NoteStore::new(paths.clone())
        .with_index(Rc::new(SearchIndex::open(&paths).expect("index reopened")));

    let mut older = store.create().expect("note created");
    older.body = "older".to_owned();
    store.save(&mut older).expect("note saved");

    let mut pinned = store.create().expect("note created");
    pinned.body = "pinned".to_owned();
    pinned.meta.pinned = true;
    store.save(&mut pinned).expect("note saved");

    let mut newer = store.create().expect("note created");
    newer.body = "newer".to_owned();
    store.save(&mut newer).expect("note saved");

    let listed = index
        .search("", TrashFilter::Exclude)
        .expect("notes listed");

    assert_eq!(listed, vec![pinned.meta.id, newer.meta.id, older.meta.id]);
}

#[test]
fn trash_filter_selects_active_and_trashed_notes() {
    let (_temp, paths) = test_paths();
    let index = Rc::new(SearchIndex::open(&paths).expect("index opened"));
    let store = NoteStore::new(paths.clone()).with_index(Rc::clone(&index));

    let mut active = store.create().expect("note created");
    active.body = "active term".to_owned();
    store.save(&mut active).expect("note saved");

    let mut trashed = store.create().expect("note created");
    trashed.body = "trashed term".to_owned();
    trashed.meta.trashed = true;
    trashed.meta.trashed_at = Some(Utc::now());
    store.save(&mut trashed).expect("note saved");

    assert_eq!(
        index.search("term", TrashFilter::Exclude).expect("search"),
        vec![active.meta.id]
    );
    assert_eq!(
        index.search("term", TrashFilter::Only).expect("search"),
        vec![trashed.meta.id]
    );
    assert_eq!(
        index.search("term", TrashFilter::Include).expect("search"),
        vec![trashed.meta.id, active.meta.id]
    );
}

#[test]
fn index_file_is_private() {
    let (_temp, paths) = test_paths();
    let _index = SearchIndex::open(&paths).expect("index opened");

    let mode = fs::metadata(&paths.index)
        .expect("index metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
}

#[test]
fn corrupt_index_is_reset_and_rebuilt() {
    let (_temp, paths) = test_paths();
    let id = seed_note(&paths, "recoverable content", &[]);
    let index = SearchIndex::open(&paths).expect("index opened");
    drop(index);

    fs::write(&paths.index, b"this is not a sqlite database").expect("corrupt index written");

    let index = SearchIndex::open(&paths).expect("index recovered");

    assert_eq!(search(&index, "recoverable"), vec![id]);
}

#[test]
fn index_remove_hides_deleted_note() {
    let (_temp, paths) = test_paths();
    let first = seed_note(&paths, "alpha removable", &[]);
    let second = seed_note(&paths, "beta retained", &[]);
    let index = SearchIndex::open(&paths).expect("index opened");

    index.remove(first).expect("note removed from index");

    assert!(search(&index, "alpha").is_empty());
    assert_eq!(search(&index, "beta"), vec![second]);
    assert_eq!(
        index.search("", TrashFilter::Exclude).expect("list"),
        vec![second]
    );
}

#[test]
fn rebuilds_and_searches_a_large_note_set() {
    let (_temp, paths) = test_paths();
    let mut target = Uuid::nil();

    for number in 0..5_000 {
        let mut note = Note::new();
        note.body = format!("generated note number {number} with searchable token");
        if number == 4_321 {
            note.body.push_str(" needle");
            target = note.meta.id;
        }
        let dir = paths::note_dir(&paths, note.meta.id);
        fs::create_dir_all(&dir).expect("note directory");
        fs::write(dir.join("note.md"), &note.body).expect("body written");
        fs::write(
            dir.join("meta.json"),
            serde_json::to_vec(&note.meta).expect("metadata serialized"),
        )
        .expect("metadata written");
    }

    let index = SearchIndex::open(&paths).expect("index opened");
    let started = std::time::Instant::now();
    let found = search(&index, "needle");
    let elapsed = started.elapsed();

    assert_eq!(found, vec![target]);
    assert_eq!(search(&index, "generated").len(), 5_000);
    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "simple search over 5,000 notes took {elapsed:?}"
    );

    let started = std::time::Instant::now();
    let summaries = index
        .search_summaries("", TrashFilter::Exclude)
        .expect("summary list");
    let list_elapsed = started.elapsed();

    assert_eq!(summaries.len(), 5_000);
    assert!(summaries.iter().any(|summary| summary.id == target));
    assert!(
        summaries
            .iter()
            .find(|summary| summary.id == target)
            .is_some_and(|summary| summary.title.contains("needle")),
        "summary titles are populated"
    );
    assert!(
        list_elapsed < std::time::Duration::from_millis(100),
        "summary list over 5,000 notes took {list_elapsed:?}"
    );
}

#[test]
fn summaries_expose_browser_row_metadata() {
    let (_temp, paths) = test_paths();
    let store = NoteStore::new(paths.clone());
    let mut note = store.create().expect("note created");
    note.body = "Project alpha kickoff\nSecond line".to_owned();
    note.meta.tags = vec!["Work".to_owned(), "Ideas".to_owned()];
    note.meta.pinned = true;
    store.save(&mut note).expect("note saved");

    let index = SearchIndex::open(&paths).expect("index opened");
    let summaries = index
        .search_summaries("alpha", TrashFilter::Exclude)
        .expect("summary search");

    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.id, note.meta.id);
    assert_eq!(summary.title, "Project alpha kickoff");
    assert_eq!(summary.tags, vec!["Work", "Ideas"]);
    assert!(summary.pinned);
    assert!(!summary.trashed);
    assert!(!summary.has_attachments);
    assert_eq!(summary.modified_at, note.meta.modified_at);
}

#[test]
fn trash_restore_and_purge_update_files_and_index() {
    let (_temp, paths) = test_paths();
    let index = Rc::new(SearchIndex::open(&paths).expect("index opened"));
    let store = NoteStore::new(paths.clone()).with_index(Rc::clone(&index));
    let mut note = store.create().expect("note created");
    note.body = "lifecycle content".to_owned();
    store.save(&mut note).expect("note saved");
    let id = note.meta.id;

    let trashed = trash::trash(&store, id).expect("note trashed");
    assert!(trashed.meta.trashed);
    assert!(trashed.meta.trashed_at.is_some());
    assert!(search(&index, "lifecycle").is_empty());
    assert_eq!(
        index
            .search("lifecycle", TrashFilter::Only)
            .expect("search"),
        vec![id]
    );

    let restored = trash::restore(&store, id).expect("note restored");
    assert!(!restored.meta.trashed);
    assert!(restored.meta.trashed_at.is_none());
    assert_eq!(search(&index, "lifecycle"), vec![id]);

    trash::trash(&store, id).expect("note trashed again");
    trash::purge(&store, id).expect("note purged");

    assert!(!paths::note_dir(&paths, id).exists());
    assert!(
        index
            .search("lifecycle", TrashFilter::Include)
            .expect("search")
            .is_empty()
    );
    assert!(matches!(store.load(id), Err(StorageError::NotFound { .. })));
}

#[test]
fn purge_refuses_to_delete_an_active_note() {
    let (_temp, paths) = test_paths();
    let store = NoteStore::new(paths.clone());
    let note = store.create().expect("note created");

    assert!(matches!(
        trash::purge(&store, note.meta.id),
        Err(StorageError::NotTrashed { .. })
    ));
    assert!(paths::note_dir(&paths, note.meta.id).exists());
}
