use libadwaita as adw;
use libadwaita::prelude::*;

use crate::storage::attachments::{FileCandidate, LARGE_FILE_THRESHOLD};

const LIST_LIMIT: usize = 5;

pub fn confirm_large_files(
    parent: &adw::ApplicationWindow,
    files: &[FileCandidate],
    on_copy: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) {
    let mut body = format!(
        "These files are larger than {} MB:\n",
        LARGE_FILE_THRESHOLD / (1024 * 1024)
    );

    for file in files.iter().take(LIST_LIMIT) {
        body.push_str(&format!(
            "- {} ({})\n",
            file.original_name,
            format_size(file.size_bytes)
        ));
    }

    if files.len() > LIST_LIMIT {
        body.push_str(&format!("- and {} more\n", files.len() - LIST_LIMIT));
    }

    body.push_str("\nCopying them into Stickies may use significant disk space.");

    let dialog = adw::AlertDialog::new(Some("Copy large files?"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("copy", "Copy Anyway");
    dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_dialog: &adw::AlertDialog, response| {
        if response == "copy" {
            on_copy();
        } else {
            on_cancel();
        }
    });
    dialog.present(Some(parent));
}

fn format_size(bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    format!("{megabytes:.1} MB")
}

pub fn confirm_remove_attachment(
    parent: &adw::ApplicationWindow,
    name: &str,
    on_remove: impl Fn() + 'static,
) {
    let body = format!("“{name}” will be removed from this note and deleted from disk.");
    let dialog = adw::AlertDialog::new(Some("Remove attachment?"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("remove", "Remove");
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_dialog: &adw::AlertDialog, response| {
        if response == "remove" {
            on_remove();
        }
    });
    dialog.present(Some(parent));
}

pub fn confirm_purge(parent: &adw::ApplicationWindow, title: &str, on_delete: impl Fn() + 'static) {
    let body = format!(
        "“{title}” will be permanently deleted, including its attachments. This cannot be undone."
    );
    let dialog = adw::AlertDialog::new(Some("Delete permanently?"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete Permanently");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_dialog: &adw::AlertDialog, response| {
        if response == "delete" {
            on_delete();
        }
    });
    dialog.present(Some(parent));
}

pub fn confirm_trash(parent: &adw::ApplicationWindow, title: &str, on_trash: impl Fn() + 'static) {
    let body = format!("“{title}” will be moved to Trash. You can restore it later.");
    let dialog = adw::AlertDialog::new(Some("Move note to Trash?"), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("trash", "Move to Trash");
    dialog.set_response_appearance("trash", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_dialog: &adw::AlertDialog, response| {
        if response == "trash" {
            on_trash();
        }
    });
    dialog.present(Some(parent));
}
