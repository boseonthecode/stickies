use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gdk4 as gdk;
use gdk4::prelude::*;
use libadwaita as adw;

use crate::storage::attachments::{
    self, FileCandidate, FileInspection, FileRejection, RejectedFile,
};
use crate::storage::draft::NoteDraft;
use crate::ui::attachment_view::AttachmentView;
use crate::ui::dialogs;
use crate::ui::note_window::{save_now, show_toast};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClipboardPaste {
    Default,
    Files,
    Image,
}

fn classify_clipboard<'a>(mime_types: impl IntoIterator<Item = &'a str>) -> ClipboardPaste {
    let mut has_files = false;
    let mut has_image = false;
    let mut has_text = false;

    for mime in mime_types {
        if mime == "text/uri-list" || mime == "x-special/gnome-copied-files" {
            has_files = true;
        }
        if mime.starts_with("image/") {
            has_image = true;
        } else if mime.starts_with("text/") {
            has_text = true;
        }
    }

    if has_files {
        ClipboardPaste::Files
    } else if has_image && !has_text {
        ClipboardPaste::Image
    } else {
        ClipboardPaste::Default
    }
}

pub(super) fn is_paste_shortcut(key: gdk::Key, state: gdk::ModifierType) -> bool {
    (key == gdk::Key::v || key == gdk::Key::V)
        && state.contains(gdk::ModifierType::CONTROL_MASK)
        && !state.contains(gdk::ModifierType::SHIFT_MASK)
}

pub(super) fn paste_from_clipboard(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
) -> glib::Propagation {
    let Some(display) = gdk::Display::default() else {
        return glib::Propagation::Proceed;
    };
    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    let mime_types = formats.mime_types();
    tracing::debug!(mime_types = ?mime_types, "clipboard formats");
    let paste = classify_clipboard(mime_types.iter().map(|mime| mime.as_str()));

    match paste {
        ClipboardPaste::Default => glib::Propagation::Proceed,
        ClipboardPaste::Image => {
            paste_image(clipboard, session, window, title, overlay, view);
            glib::Propagation::Stop
        }
        ClipboardPaste::Files => {
            paste_files(clipboard, session, window, title, overlay, view);
            glib::Propagation::Stop
        }
    }
}

fn paste_image(
    clipboard: gdk::Clipboard,
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
) {
    title.set_subtitle("Pasting image…");

    let paste_session = Rc::clone(session);
    let paste_window = window.clone();
    let paste_title = title.clone();
    let paste_overlay = overlay.clone();
    let paste_view = view.clone();
    clipboard.read_texture_async(None::<&gio::Cancellable>, move |result| {
        let texture = match result {
            Ok(Some(texture)) => texture,
            Ok(None) => {
                paste_title.set_subtitle("");
                return;
            }
            Err(_) => {
                tracing::error!("failed to read image from clipboard");
                paste_title.set_subtitle("Paste failed");
                show_toast(
                    &paste_overlay,
                    "Could not read the image from the clipboard.",
                );
                return;
            }
        };

        attach_texture(
            &texture,
            &paste_session,
            &paste_window,
            &paste_title,
            &paste_overlay,
            &paste_view,
        );
    });
}

pub(super) fn attach_texture(
    texture: &gdk::Texture,
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
) {
    let bytes = texture.save_to_png_bytes();
    let added = session.borrow_mut().add_png(bytes.as_ref());
    match added {
        Ok(_) => {
            {
                let draft = session.borrow();
                refresh_attachment_view(view, &draft);
            }
            save_now(session, window, title, overlay);
        }
        Err(_) => {
            tracing::error!("failed to store pasted image");
            title.set_subtitle("Paste failed");
            show_toast(overlay, "Could not save the pasted image.");
        }
    }
}

fn paste_files(
    clipboard: gdk::Clipboard,
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
) {
    title.set_subtitle("Pasting files…");

    let paste_session = Rc::clone(session);
    let paste_window = window.clone();
    let paste_title = title.clone();
    let paste_overlay = overlay.clone();
    let paste_view = view.clone();
    clipboard.read_value_async(
        gdk::FileList::static_type(),
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
            let value = match result {
                Ok(value) => value,
                Err(_) => {
                    tracing::error!("failed to read file list from clipboard");
                    paste_title.set_subtitle("Paste failed");
                    show_toast(&paste_overlay, "Could not read files from the clipboard.");
                    return;
                }
            };
            let file_list = match value.get::<gdk::FileList>() {
                Ok(file_list) => file_list,
                Err(_) => {
                    tracing::error!("clipboard did not contain a file list");
                    paste_title.set_subtitle("Paste failed");
                    show_toast(&paste_overlay, "Could not read files from the clipboard.");
                    return;
                }
            };

            let (paths, rejected) = local_file_paths(&file_list);
            let mut inspection = attachments::inspect_files(paths);
            inspection.rejected.extend(rejected);

            finish_file_paste(
                &paste_session,
                &paste_window,
                &paste_title,
                &paste_overlay,
                &paste_view,
                inspection,
            );
        },
    );
}

pub(super) fn local_file_paths(file_list: &gdk::FileList) -> (Vec<PathBuf>, Vec<RejectedFile>) {
    let mut paths = Vec::new();
    let mut rejected = Vec::new();

    for file in file_list.files() {
        match file.path() {
            Some(path) => paths.push(path),
            None => rejected.push(RejectedFile {
                original_name: "remote file".to_owned(),
                reason: FileRejection::NotLocal,
            }),
        }
    }

    (paths, rejected)
}

pub(super) fn finish_file_paste(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
    inspection: FileInspection,
) {
    if inspection.files.is_empty() {
        title.set_subtitle("");
        show_toast(overlay, &file_report_message(0, &inspection.rejected));
        return;
    }

    if inspection.needs_large_confirmation() {
        let large_files: Vec<FileCandidate> =
            inspection.large_files().into_iter().cloned().collect();
        let session = Rc::clone(session);
        let window = window.clone();
        let dialog_parent = window.clone();
        let title = title.clone();
        let overlay = overlay.clone();
        let view = view.clone();

        title.set_subtitle("");
        let cancel_overlay = overlay.clone();
        dialogs::confirm_large_files(
            &dialog_parent,
            &large_files,
            move || {
                copy_files(&session, &window, &title, &overlay, &view, &inspection);
            },
            move || show_toast(&cancel_overlay, "Large file copy canceled."),
        );
    } else {
        copy_files(session, window, title, overlay, view, &inspection);
    }
}

fn copy_files(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    view: &AttachmentView,
    inspection: &FileInspection,
) {
    let report = match session.borrow_mut().add_files(&inspection.files) {
        Ok(report) => report,
        Err(_) => {
            tracing::error!("failed to store pasted files");
            title.set_subtitle("Paste failed");
            show_toast(overlay, "Could not save the pasted files.");
            return;
        }
    };

    {
        let draft = session.borrow();
        refresh_attachment_view(view, &draft);
    }

    let saved = if report.imported.is_empty() {
        title.set_subtitle("");
        true
    } else {
        save_now(session, window, title, overlay)
    };

    let mut all_rejected = inspection.rejected.clone();
    all_rejected.extend(report.rejected);

    if saved && (!all_rejected.is_empty() || report.imported.is_empty()) {
        show_toast(
            overlay,
            &file_report_message(report.imported.len(), &all_rejected),
        );
    }
}

fn file_report_message(imported: usize, rejected: &[RejectedFile]) -> String {
    let mut reasons = Vec::new();

    for file in rejected {
        let reason = file.reason.label();
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    }

    if reasons.is_empty() {
        format!("Attached {imported} file(s).")
    } else if imported == 0 {
        format!("No files attached: {}.", reasons.join("; "))
    } else {
        format!(
            "Attached {imported} file(s). Skipped {} file(s): {}.",
            rejected.len(),
            reasons.join("; ")
        )
    }
}

pub(super) fn refresh_attachment_view(view: &AttachmentView, draft: &NoteDraft) {
    view.refresh(&draft.attachment_entries());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_image_only_clipboard_for_attachment_paste() {
        assert_eq!(
            classify_clipboard(["image/png", "image/jpeg"]),
            ClipboardPaste::Image
        );
    }

    #[test]
    fn prefers_default_paste_when_text_is_available() {
        assert_eq!(
            classify_clipboard(["image/png", "text/plain"]),
            ClipboardPaste::Default
        );
        assert_eq!(classify_clipboard(["text/plain"]), ClipboardPaste::Default);
    }

    #[test]
    fn file_lists_take_precedence_over_other_formats() {
        assert_eq!(
            classify_clipboard(["text/uri-list", "text/plain"]),
            ClipboardPaste::Files
        );
        assert_eq!(
            classify_clipboard(["x-special/gnome-copied-files", "image/png"]),
            ClipboardPaste::Files
        );
    }

    #[test]
    fn unsupported_clipboard_types_fall_through() {
        assert_eq!(
            classify_clipboard(["application/pdf", "text/html"]),
            ClipboardPaste::Default
        );
        assert_eq!(
            classify_clipboard(Vec::<&str>::new()),
            ClipboardPaste::Default
        );
    }

    #[test]
    fn distinguishes_ctrl_v_from_ctrl_shift_v_and_plain_v() {
        assert!(is_paste_shortcut(
            gdk::Key::v,
            gdk::ModifierType::CONTROL_MASK
        ));
        assert!(!is_paste_shortcut(
            gdk::Key::v,
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
        ));
        assert!(!is_paste_shortcut(gdk::Key::v, gdk::ModifierType::empty()));
    }

    #[test]
    fn reports_unique_skip_reasons() {
        let rejected = vec![
            RejectedFile {
                original_name: "folder".to_owned(),
                reason: FileRejection::Directory,
            },
            RejectedFile {
                original_name: "another folder".to_owned(),
                reason: FileRejection::Directory,
            },
            RejectedFile {
                original_name: "link".to_owned(),
                reason: FileRejection::Symlink,
            },
        ];

        assert_eq!(
            file_report_message(2, &rejected),
            "Attached 2 file(s). Skipped 3 file(s): folders cannot be attached; symlinks cannot be attached."
        );
        assert_eq!(
            file_report_message(0, &rejected),
            "No files attached: folders cannot be attached; symlinks cannot be attached."
        );
        assert_eq!(file_report_message(2, &[]), "Attached 2 file(s).");
    }
}
