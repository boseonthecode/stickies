use std::cell::RefCell;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, Transaction, params};
use uuid::Uuid;

use crate::config::DataPaths;
use crate::error::IndexError;
use crate::util::text::note_title;

use super::note::{Note, NoteStore};
use super::tags;

pub const INDEX_SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
    note_id         TEXT PRIMARY KEY,
    created_at      TEXT NOT NULL,
    modified_at     TEXT NOT NULL,
    pinned          INTEGER NOT NULL DEFAULT 0,
    trashed         INTEGER NOT NULL DEFAULT 0,
    trashed_at      TEXT,
    tags            TEXT NOT NULL DEFAULT '',
    has_attachments INTEGER NOT NULL DEFAULT 0,
    snippet         TEXT NOT NULL DEFAULT '',
    title           TEXT NOT NULL DEFAULT ''
);

CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
    note_id UNINDEXED,
    body,
    tags,
    attachment_names,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS attachments (
    attachment_id TEXT PRIMARY KEY,
    note_id       TEXT NOT NULL,
    stored_name   TEXT NOT NULL,
    original_name TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    kind          TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    FOREIGN KEY (note_id) REFERENCES notes(note_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_notes_listing
    ON notes (trashed, pinned DESC, modified_at DESC);

CREATE INDEX IF NOT EXISTS idx_notes_modified
    ON notes (modified_at DESC);

CREATE INDEX IF NOT EXISTS idx_attachments_note
    ON attachments (note_id);

CREATE INDEX IF NOT EXISTS idx_attachments_original_name
    ON attachments (original_name);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashFilter {
    Exclude,
    Only,
    Include,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: Uuid,
    pub modified_at: DateTime<Utc>,
    pub pinned: bool,
    pub trashed: bool,
    pub trashed_at: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
    pub has_attachments: bool,
    pub title: String,
}

impl NoteSummary {
    pub fn from_note(note: &Note) -> Self {
        Self {
            id: note.meta.id,
            modified_at: note.meta.modified_at,
            pinned: note.meta.pinned,
            trashed: note.meta.trashed,
            trashed_at: note.meta.trashed_at,
            tags: tags::normalize_tags(&note.meta.tags),
            has_attachments: !note.meta.attachments.is_empty(),
            title: note_title(&note.body),
        }
    }
}

pub struct SearchIndex {
    paths: DataPaths,
    path: PathBuf,
    conn: RefCell<Connection>,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchIndex")
            .field("path", &self.path)
            .finish()
    }
}

impl SearchIndex {
    pub fn open(paths: &DataPaths) -> Result<Self, IndexError> {
        paths.ensure_dirs()?;
        let path = paths.index.clone();

        if !path.exists() {
            let index = Self::create(paths)?;
            index.rebuild()?;
            return Ok(index);
        }

        match Self::open_existing(paths) {
            Ok(index) => match index.needs_rebuild() {
                Ok(false) => Ok(index),
                Ok(true) => {
                    drop(index);
                    Self::reset(&path)?;
                    let index = Self::create(paths)?;
                    index.rebuild()?;
                    Ok(index)
                }
                Err(IndexError::Storage(error)) => Err(IndexError::Storage(error)),
                Err(_) => {
                    drop(index);
                    Self::reset(&path)?;
                    let index = Self::create(paths)?;
                    index.rebuild()?;
                    Ok(index)
                }
            },
            Err(_) => {
                Self::reset(&path)?;
                let index = Self::create(paths)?;
                index.rebuild()?;
                Ok(index)
            }
        }
    }

    pub fn rebuild(&self) -> Result<(), IndexError> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn
            .transaction()
            .map_err(|source| self.database_error(source))?;

        tx.execute("DELETE FROM notes_fts", [])
            .map_err(|source| self.database_error(source))?;
        tx.execute("DELETE FROM attachments", [])
            .map_err(|source| self.database_error(source))?;
        tx.execute("DELETE FROM notes", [])
            .map_err(|source| self.database_error(source))?;

        let store = NoteStore::new(self.paths.clone());
        for id in store.list_ids()? {
            match store.load(id) {
                Ok(note) => {
                    upsert_note(&tx, &note).map_err(|source| self.database_error(source))?
                }
                Err(error) => {
                    tracing::warn!(note_id = %id, %error, "skipping unreadable note during index rebuild");
                }
            }
        }

        tx.commit().map_err(|source| self.database_error(source))
    }

    pub fn upsert(&self, note: &Note) -> Result<(), IndexError> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn
            .transaction()
            .map_err(|source| self.database_error(source))?;

        upsert_note(&tx, note).map_err(|source| self.database_error(source))?;
        tx.commit().map_err(|source| self.database_error(source))
    }

    pub fn remove(&self, note_id: Uuid) -> Result<(), IndexError> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn
            .transaction()
            .map_err(|source| self.database_error(source))?;
        let id = note_id.to_string();

        tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id])
            .map_err(|source| self.database_error(source))?;
        tx.execute("DELETE FROM notes WHERE note_id = ?1", params![id])
            .map_err(|source| self.database_error(source))?;
        tx.commit().map_err(|source| self.database_error(source))
    }

    pub fn search(&self, query: &str, filter: TrashFilter) -> Result<Vec<Uuid>, IndexError> {
        Ok(self
            .search_summaries(query, filter)?
            .into_iter()
            .map(|summary| summary.id)
            .collect())
    }

    pub fn search_summaries(
        &self,
        query: &str,
        filter: TrashFilter,
    ) -> Result<Vec<NoteSummary>, IndexError> {
        let query = query.trim();
        if query.is_empty() {
            return self.list_summaries(filter);
        }

        let Some(fts_query) = fts_query(query) else {
            return Ok(Vec::new());
        };

        self.search_fts_summaries(&fts_query, filter)
            .or_else(|_| self.search_like_summaries(query, filter))
    }

    fn list_summaries(&self, filter: TrashFilter) -> Result<Vec<NoteSummary>, IndexError> {
        let conn = self.conn.borrow();
        let sql = format!(
            "SELECT {} FROM notes WHERE 1 = 1{} ORDER BY pinned DESC, modified_at DESC",
            SUMMARY_COLUMNS,
            trash_clause(filter)
        );
        let mut statement = conn
            .prepare(&sql)
            .map_err(|source| self.database_error(source))?;
        let rows = statement
            .query_map([], summary_from_row)
            .map_err(|source| self.database_error(source))?;

        collect_summaries(rows, &self.path)
    }

    fn search_fts_summaries(
        &self,
        query: &str,
        filter: TrashFilter,
    ) -> Result<Vec<NoteSummary>, IndexError> {
        let conn = self.conn.borrow();
        let sql = format!(
            "SELECT {} FROM notes_fts              JOIN notes ON notes.note_id = notes_fts.note_id              WHERE notes_fts MATCH ?1{}              ORDER BY bm25(notes_fts), notes.modified_at DESC",
            SUMMARY_COLUMNS,
            trash_clause(filter)
        );
        let mut statement = conn
            .prepare(&sql)
            .map_err(|source| self.database_error(source))?;
        let rows = statement
            .query_map(params![query], summary_from_row)
            .map_err(|source| self.database_error(source))?;

        collect_summaries(rows, &self.path)
    }

    fn search_like_summaries(
        &self,
        query: &str,
        filter: TrashFilter,
    ) -> Result<Vec<NoteSummary>, IndexError> {
        let conn = self.conn.borrow();
        let sql = format!(
            "SELECT {} FROM notes_fts              JOIN notes ON notes.note_id = notes_fts.note_id              LEFT JOIN attachments ON attachments.note_id = notes.note_id              WHERE (instr(lower(notes_fts.body), lower(?1)) > 0                 OR instr(lower(notes_fts.tags), lower(?1)) > 0                 OR instr(lower(notes_fts.attachment_names), lower(?1)) > 0                 OR instr(lower(attachments.original_name), lower(?1)) > 0){}              GROUP BY notes.note_id              ORDER BY notes.pinned DESC, notes.modified_at DESC",
            SUMMARY_COLUMNS,
            trash_clause(filter)
        );
        let mut statement = conn
            .prepare(&sql)
            .map_err(|source| self.database_error(source))?;
        let rows = statement
            .query_map(params![query], summary_from_row)
            .map_err(|source| self.database_error(source))?;

        collect_summaries(rows, &self.path)
    }

    fn create(paths: &DataPaths) -> Result<Self, IndexError> {
        let path = paths.index.clone();
        let conn = Self::open_connection(&path)?;
        conn.execute_batch(SCHEMA)
            .map_err(|source| IndexError::Database {
                path: path.clone(),
                source,
            })?;
        conn.execute_batch(&format!("PRAGMA user_version = {INDEX_SCHEMA_VERSION};"))
            .map_err(|source| IndexError::Database {
                path: path.clone(),
                source,
            })?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
            params![INDEX_SCHEMA_VERSION.to_string()],
        )
        .map_err(|source| IndexError::Database {
            path: path.clone(),
            source,
        })?;

        Ok(Self {
            paths: paths.clone(),
            path,
            conn: RefCell::new(conn),
        })
    }

    fn open_existing(paths: &DataPaths) -> Result<Self, IndexError> {
        let path = paths.index.clone();
        let conn = Self::open_connection(&path)?;

        Ok(Self {
            paths: paths.clone(),
            path,
            conn: RefCell::new(conn),
        })
    }

    fn open_connection(path: &Path) -> Result<Connection, IndexError> {
        let conn = Connection::open(path).map_err(|source| IndexError::Database {
            path: path.to_path_buf(),
            source,
        })?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|source| IndexError::Database {
                path: path.to_path_buf(),
                source,
            })?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .map_err(|source| IndexError::Database {
                path: path.to_path_buf(),
                source,
            })?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            IndexError::SetPermissions {
                path: path.to_path_buf(),
                source,
            }
        })?;

        Ok(conn)
    }

    fn needs_rebuild(&self) -> Result<bool, IndexError> {
        let conn = self.conn.borrow();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|source| self.database_error(source))?;

        if version != INDEX_SCHEMA_VERSION {
            return Ok(true);
        }

        let check: String = conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|source| self.database_error(source))?;
        if check != "ok" {
            return Ok(true);
        }

        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master \
                 WHERE type = 'table' \
                   AND name IN ('schema_meta', 'notes', 'notes_fts', 'attachments')",
                [],
                |row| row.get(0),
            )
            .map_err(|source| self.database_error(source))?;
        if tables != 4 {
            return Ok(true);
        }

        let indexed: i64 = conn
            .query_row("SELECT count(*) FROM notes", [], |row| row.get(0))
            .map_err(|source| self.database_error(source))?;
        let expected = NoteStore::new(self.paths.clone()).list_ids()?.len() as i64;

        Ok(indexed != expected)
    }

    fn reset(path: &Path) -> Result<(), IndexError> {
        for candidate in [
            path.to_path_buf(),
            sidecar_path(path, "-wal"),
            sidecar_path(path, "-shm"),
        ] {
            match fs::remove_file(&candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(IndexError::Reset {
                        path: candidate,
                        source,
                    });
                }
            }
        }

        Ok(())
    }

    fn database_error(&self, source: rusqlite::Error) -> IndexError {
        IndexError::Database {
            path: self.path.clone(),
            source,
        }
    }
}

fn upsert_note(tx: &Transaction<'_>, note: &Note) -> rusqlite::Result<()> {
    let note_id = note.meta.id.to_string();
    let tags = normalized_tags(&note.meta.tags);
    let attachment_names = note
        .meta
        .attachments
        .iter()
        .map(|attachment| attachment.original_name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let snippet = snippet(&note.body);

    tx.execute(
        "INSERT INTO notes (
            note_id, created_at, modified_at, pinned, trashed, trashed_at,
            tags, has_attachments, snippet, title
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(note_id) DO UPDATE SET
            created_at = excluded.created_at,
            modified_at = excluded.modified_at,
            pinned = excluded.pinned,
            trashed = excluded.trashed,
            trashed_at = excluded.trashed_at,
            tags = excluded.tags,
            has_attachments = excluded.has_attachments,
            snippet = excluded.snippet,
            title = excluded.title",
        params![
            note_id,
            note.meta.created_at.to_rfc3339(),
            note.meta.modified_at.to_rfc3339(),
            note.meta.pinned as i64,
            note.meta.trashed as i64,
            note.meta.trashed_at.map(|time| time.to_rfc3339()),
            tags,
            !note.meta.attachments.is_empty() as i64,
            snippet,
            note_title(&note.body),
        ],
    )?;

    tx.execute(
        "DELETE FROM attachments WHERE note_id = ?1",
        params![note.meta.id.to_string()],
    )?;

    for attachment in &note.meta.attachments {
        tx.execute(
            "INSERT INTO attachments (
                attachment_id, note_id, stored_name, original_name, mime,
                size_bytes, kind, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                attachment.id.to_string(),
                note.meta.id.to_string(),
                attachment.stored_name,
                attachment.original_name,
                attachment.mime,
                attachment.size_bytes as i64,
                attachment_kind(&attachment.kind),
                attachment.created_at.to_rfc3339(),
            ],
        )?;
    }

    tx.execute(
        "DELETE FROM notes_fts WHERE note_id = ?1",
        params![note.meta.id.to_string()],
    )?;
    tx.execute(
        "INSERT INTO notes_fts (note_id, body, tags, attachment_names) \
         VALUES (?1, ?2, ?3, ?4)",
        params![note.meta.id.to_string(), note.body, tags, attachment_names],
    )?;

    Ok(())
}

const SUMMARY_COLUMNS: &str = "notes.note_id, notes.modified_at, notes.pinned, \
    notes.trashed, notes.trashed_at, notes.tags, notes.has_attachments, notes.title";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<NoteSummary> {
    let note_id: String = row.get(0)?;
    let modified_at: String = row.get(1)?;
    let pinned: i64 = row.get(2)?;
    let trashed: i64 = row.get(3)?;
    let trashed_at: Option<String> = row.get(4)?;
    let tags: String = row.get(5)?;
    let has_attachments: i64 = row.get(6)?;
    let title: String = row.get(7)?;

    Ok(NoteSummary {
        id: Uuid::parse_str(&note_id).unwrap_or(Uuid::nil()),
        modified_at: parse_timestamp(&modified_at)?,
        pinned: pinned != 0,
        trashed: trashed != 0,
        trashed_at: trashed_at.as_deref().map(parse_timestamp).transpose()?,
        tags: tags.split_whitespace().map(str::to_owned).collect(),
        has_attachments: has_attachments != 0,
        title,
    })
}

fn parse_timestamp(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(source),
            )
        })
}

fn collect_summaries(
    rows: rusqlite::MappedRows<'_, impl FnMut(&Row<'_>) -> rusqlite::Result<NoteSummary>>,
    path: &Path,
) -> Result<Vec<NoteSummary>, IndexError> {
    let mut summaries = Vec::new();

    for row in rows {
        let summary = row.map_err(|source| IndexError::Database {
            path: path.to_path_buf(),
            source,
        })?;
        if !summary.id.is_nil() {
            summaries.push(summary);
        }
    }

    Ok(summaries)
}

fn trash_clause(filter: TrashFilter) -> &'static str {
    match filter {
        TrashFilter::Exclude => " AND notes.trashed = 0",
        TrashFilter::Only => " AND notes.trashed = 1",
        TrashFilter::Include => "",
    }
}

fn fts_query(query: &str) -> Option<String> {
    let mut tokens = Vec::new();

    for token in query.split(|character: char| !(character.is_alphanumeric() || character == '_')) {
        if !token.is_empty() {
            tokens.push(token.to_owned());
        }
    }

    let (last, rest) = tokens.split_last()?;
    let mut terms = rest.iter().map(String::as_str).collect::<Vec<_>>();
    let prefixed = format!("{last}*");
    terms.push(&prefixed);

    Some(terms.join(" "))
}

fn normalized_tags(tags: &[String]) -> String {
    tags::normalize_tags(tags).join(" ")
}

fn snippet(body: &str) -> String {
    body.split_whitespace()
        .take(40)
        .collect::<Vec<_>>()
        .join(" ")
}

fn attachment_kind(kind: &super::note::AttachmentKind) -> &'static str {
    match kind {
        super::note::AttachmentKind::Image => "image",
        super::note::AttachmentKind::File => "file",
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}
