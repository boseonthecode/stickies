use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::DataPaths;
use crate::error::StorageError;

use super::note::{AttachmentKind, AttachmentMeta};
use super::{atomic, paths};

pub const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    pub path: PathBuf,
    pub original_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRejection {
    Directory,
    Symlink,
    NotLocal,
    Unreadable,
}

impl FileRejection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Directory => "folders cannot be attached",
            Self::Symlink => "symlinks cannot be attached",
            Self::NotLocal => "remote files cannot be attached",
            Self::Unreadable => "some files could not be read",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedFile {
    pub original_name: String,
    pub reason: FileRejection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInspection {
    pub files: Vec<FileCandidate>,
    pub rejected: Vec<RejectedFile>,
}

impl FileInspection {
    pub fn needs_large_confirmation(&self) -> bool {
        self.files
            .iter()
            .any(|file| file.size_bytes > LARGE_FILE_THRESHOLD)
    }

    pub fn large_files(&self) -> Vec<&FileCandidate> {
        self.files
            .iter()
            .filter(|file| file.size_bytes > LARGE_FILE_THRESHOLD)
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileImportReport {
    pub imported: Vec<AttachmentMeta>,
    pub rejected: Vec<RejectedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentEntry {
    pub attachment: AttachmentMeta,
    pub path: Option<PathBuf>,
    pub missing: bool,
}

pub fn attachment_entry(
    data_paths: &DataPaths,
    note_id: Uuid,
    attachment: &AttachmentMeta,
) -> AttachmentEntry {
    let path = paths::attachment_path(data_paths, note_id, &attachment.stored_name);
    let missing = path
        .as_deref()
        .map(|path| !source_is_regular_file(path))
        .unwrap_or(true);

    AttachmentEntry {
        attachment: attachment.clone(),
        path,
        missing,
    }
}

pub fn pasted_image_original_name(at: DateTime<Utc>) -> String {
    format!("pasted-image-{}.png", at.format("%Y%m%d-%H%M%S"))
}

pub fn import_png_bytes(
    data_paths: &DataPaths,
    note_id: Uuid,
    bytes: &[u8],
) -> Result<AttachmentMeta, StorageError> {
    if bytes.is_empty() {
        return Err(StorageError::EmptyAttachment);
    }

    let now = Utc::now();
    let id = Uuid::now_v7();
    let stored_name = format!("{id}.png");
    let directory = paths::attachments_dir(data_paths, note_id);
    paths::create_private_dir(&directory)?;
    atomic::write(&directory.join(&stored_name), bytes)?;

    Ok(AttachmentMeta {
        id,
        stored_name,
        original_name: pasted_image_original_name(now),
        mime: "image/png".to_owned(),
        size_bytes: bytes.len() as u64,
        kind: AttachmentKind::Image,
        created_at: now,
    })
}

pub fn sanitize_original_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("attachment");
    let sanitized: String = raw
        .chars()
        .map(|character| {
            if character.is_control() || character == '/' || character == '\\' {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = sanitized.trim();
    let limited: String = trimmed.chars().take(200).collect();

    if limited.is_empty() || limited == "." || limited == ".." {
        "attachment".to_owned()
    } else {
        limited
    }
}

pub fn inspect_files(paths: impl IntoIterator<Item = PathBuf>) -> FileInspection {
    let mut inspection = FileInspection::default();

    for path in paths {
        let original_name = sanitize_original_name(&path);

        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                inspection.rejected.push(RejectedFile {
                    original_name,
                    reason: FileRejection::Symlink,
                });
            }
            Ok(metadata) if metadata.is_dir() => {
                inspection.rejected.push(RejectedFile {
                    original_name,
                    reason: FileRejection::Directory,
                });
            }
            Ok(metadata) if !metadata.is_file() => {
                inspection.rejected.push(RejectedFile {
                    original_name,
                    reason: FileRejection::Unreadable,
                });
            }
            Ok(metadata) => {
                inspection.files.push(FileCandidate {
                    path,
                    original_name,
                    size_bytes: metadata.len(),
                });
            }
            Err(_) => {
                inspection.rejected.push(RejectedFile {
                    original_name,
                    reason: FileRejection::Unreadable,
                });
            }
        }
    }

    inspection
}

pub fn import_files(
    data_paths: &DataPaths,
    note_id: Uuid,
    files: &[FileCandidate],
) -> Result<FileImportReport, StorageError> {
    let mut report = FileImportReport::default();

    if files.is_empty() {
        return Ok(report);
    }

    let directory = paths::attachments_dir(data_paths, note_id);
    paths::create_private_dir(&directory)?;

    for file in files {
        if !source_is_regular_file(&file.path) {
            report.rejected.push(RejectedFile {
                original_name: file.original_name.clone(),
                reason: FileRejection::Unreadable,
            });
            continue;
        }

        let id = Uuid::now_v7();
        let stored_name = format!("{id}{}", safe_extension(&file.path));
        let destination = directory.join(&stored_name);

        if atomic::copy(&file.path, &destination).is_err() {
            report.rejected.push(RejectedFile {
                original_name: file.original_name.clone(),
                reason: FileRejection::Unreadable,
            });
            continue;
        }

        let now = Utc::now();
        let mime = mime_guess::from_path(&file.path)
            .first_or_octet_stream()
            .essence_str()
            .to_owned();
        let kind = if mime.starts_with("image/") {
            AttachmentKind::Image
        } else {
            AttachmentKind::File
        };
        let size_bytes = fs::metadata(&destination)
            .map(|metadata| metadata.len())
            .unwrap_or(file.size_bytes);

        report.imported.push(AttachmentMeta {
            id,
            stored_name,
            original_name: file.original_name.clone(),
            mime,
            size_bytes,
            kind,
            created_at: now,
        });
    }

    Ok(report)
}

fn source_is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn safe_extension(path: &Path) -> String {
    let Some(extension) = path.extension().and_then(OsStr::to_str) else {
        return String::new();
    };
    let extension: String = extension
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(10)
        .collect();

    if extension.is_empty() {
        String::new()
    } else {
        format!(".{}", extension.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use chrono::TimeZone;
    use tempfile::TempDir;

    use super::*;

    fn paths() -> (TempDir, DataPaths) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let paths = DataPaths::with_root(temp.path().join("stickies"));
        paths.ensure_dirs().expect("data directories");
        (temp, paths)
    }

    #[test]
    fn generated_original_name_contains_timestamp_and_png_extension() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 11, 4, 19, 30, 5)
            .single()
            .expect("valid timestamp");

        assert_eq!(
            pasted_image_original_name(timestamp),
            "pasted-image-20261104-193005.png"
        );
    }

    #[test]
    fn imports_png_bytes_and_writes_private_file() {
        let (_temp, paths) = paths();
        let note_id = Uuid::now_v7();
        let bytes = [0x89, b'P', b'N', b'G', 1, 2, 3];

        let attachment = import_png_bytes(&paths, note_id, &bytes).expect("attachment imported");

        assert_eq!(attachment.mime, "image/png");
        assert_eq!(attachment.kind, AttachmentKind::Image);
        assert_eq!(attachment.size_bytes, bytes.len() as u64);
        assert!(attachment.stored_name.ends_with(".png"));
        assert!(!attachment.stored_name.contains('/'));
        assert!(attachment.original_name.starts_with("pasted-image-"));

        let path = paths::attachments_dir(&paths, note_id).join(&attachment.stored_name);
        assert_eq!(fs::read(path.clone()).expect("attachment read"), bytes);
        assert_eq!(
            fs::metadata(path)
                .expect("attachment metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn rejects_empty_png_bytes() {
        let (_temp, paths) = paths();
        let note_id = Uuid::now_v7();

        assert!(matches!(
            import_png_bytes(&paths, note_id, &[]),
            Err(StorageError::EmptyAttachment)
        ));
        assert!(!paths::attachments_dir(&paths, note_id).exists());
    }

    #[test]
    fn sanitizes_control_and_separator_characters() {
        assert_eq!(
            sanitize_original_name(Path::new("folder/evil\nname\\file.txt")),
            "evil_name_file.txt"
        );
        assert_eq!(sanitize_original_name(Path::new("/")), "attachment");
        assert_eq!(sanitize_original_name(Path::new("..")), "attachment");
    }

    #[test]
    fn inspects_regular_files_and_rejects_directories_symlinks_and_missing_paths() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let file = temp.path().join("note.txt");
        fs::write(&file, b"hello").expect("source file");
        let directory = temp.path().join("folder");
        fs::create_dir(&directory).expect("directory");
        let link = temp.path().join("link.txt");
        symlink(&file, &link).expect("symlink");
        let missing = temp.path().join("missing.txt");

        let inspection = inspect_files(vec![file.clone(), directory, link, missing]);

        assert_eq!(inspection.files.len(), 1);
        assert_eq!(inspection.files[0].path, file);
        assert_eq!(inspection.files[0].size_bytes, 5);
        assert_eq!(
            inspection
                .rejected
                .iter()
                .map(|file| file.reason)
                .collect::<Vec<_>>(),
            vec![
                FileRejection::Directory,
                FileRejection::Symlink,
                FileRejection::Unreadable
            ]
        );
    }

    #[test]
    fn flags_files_above_the_large_file_threshold() {
        let inspection = FileInspection {
            files: vec![
                FileCandidate {
                    path: PathBuf::from("/small"),
                    original_name: "small".to_owned(),
                    size_bytes: LARGE_FILE_THRESHOLD,
                },
                FileCandidate {
                    path: PathBuf::from("/large"),
                    original_name: "large".to_owned(),
                    size_bytes: LARGE_FILE_THRESHOLD + 1,
                },
            ],
            rejected: Vec::new(),
        };

        assert!(inspection.needs_large_confirmation());
        assert_eq!(inspection.large_files().len(), 1);
        assert_eq!(inspection.large_files()[0].original_name, "large");
    }

    #[test]
    fn imports_duplicate_names_with_distinct_stored_names() {
        let (temp, paths) = paths();
        let note_id = Uuid::now_v7();
        let first_dir = temp.path().join("first");
        let second_dir = temp.path().join("second");
        fs::create_dir_all(&first_dir).expect("first directory");
        fs::create_dir_all(&second_dir).expect("second directory");
        let first = first_dir.join("same.txt");
        let second = second_dir.join("same.txt");
        fs::write(&first, b"first").expect("first file");
        fs::write(&second, b"second").expect("second file");

        let inspection = inspect_files(vec![first, second]);
        let report = import_files(&paths, note_id, &inspection.files).expect("files imported");

        assert_eq!(report.imported.len(), 2);
        assert_eq!(report.imported[0].original_name, "same.txt");
        assert_eq!(report.imported[1].original_name, "same.txt");
        assert_ne!(
            report.imported[0].stored_name,
            report.imported[1].stored_name
        );
        assert_eq!(report.imported[0].kind, AttachmentKind::File);
        assert_eq!(report.imported[0].mime, "text/plain");

        let directory = paths::attachments_dir(&paths, note_id);
        for attachment in &report.imported {
            let path = directory.join(&attachment.stored_name);
            assert!(path.exists());
            assert_eq!(
                fs::metadata(path)
                    .expect("attachment metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn import_revalidates_source_paths() {
        let (temp, paths) = paths();
        let note_id = Uuid::now_v7();
        let directory = temp.path().join("folder");
        fs::create_dir(&directory).expect("directory");
        let candidate = FileCandidate {
            path: directory,
            original_name: "folder".to_owned(),
            size_bytes: 0,
        };

        let report = import_files(&paths, note_id, &[candidate]).expect("import attempted");

        assert!(report.imported.is_empty());
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0].reason, FileRejection::Unreadable);
    }
}
