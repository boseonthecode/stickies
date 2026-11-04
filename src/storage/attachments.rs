use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::DataPaths;
use crate::error::StorageError;

use super::note::{AttachmentKind, AttachmentMeta};
use super::{atomic, paths};

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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

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
}
