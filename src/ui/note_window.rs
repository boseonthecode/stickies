use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gdk4 as gdk;
use gdk4::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use sourceview5 as source;
use sourceview5::prelude::*;
use uuid::Uuid;

use crate::error::StorageError;
use crate::storage::draft::{NoteDraft, SaveOutcome};
use crate::storage::note::NoteStore;

pub const WIDGET_NAME: &str = "note-window";

const AUTOSAVE_DELAY: Duration = Duration::from_millis(650);
const SAVE_ERROR_MESSAGE: &str = "Could not save note. Check disk space and permissions.";

pub fn existing_widget_name(id: Uuid) -> String {
    format!("{WIDGET_NAME}:{id}")
}

pub fn present(app: &adw::Application, number: u64, store: NoteStore) {
    present_draft(app, number, NoteDraft::new(store));
}

pub fn present_existing(
    app: &adw::Application,
    number: u64,
    store: NoteStore,
    id: Uuid,
) -> Result<(), StorageError> {
    let draft = NoteDraft::existing(store, id)?;
    present_draft(app, number, draft);
    Ok(())
}

fn present_draft(app: &adw::Application, number: u64, draft: NoteDraft) {
    let title = format!("Note {number}");
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&title)
        .default_width(440)
        .default_height(360)
        .build();
    if draft.is_persisted() {
        window.set_widget_name(&existing_widget_name(draft.id()));
    } else {
        window.set_widget_name(WIDGET_NAME);
    }

    let title_widget = adw::WindowTitle::new(&title, "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title_widget));

    let buffer = source::Buffer::new(None);
    if let Some(language) = source::LanguageManager::default().language("markdown") {
        buffer.set_language(Some(&language));
    }
    buffer.set_highlight_syntax(true);

    let editor = source::View::with_buffer(&buffer);
    editor.set_wrap_mode(gtk::WrapMode::WordChar);
    editor.set_highlight_current_line(true);
    editor.set_top_margin(12);
    editor.set_bottom_margin(12);
    editor.set_left_margin(12);
    editor.set_right_margin(12);
    editor.add_css_class("monospace");

    let editor_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&editor)
        .vexpand(true)
        .build();

    let attachments_title = gtk::Label::new(Some("Attachments"));
    attachments_title.set_halign(gtk::Align::Start);
    attachments_title.add_css_class("heading");

    let attachments_placeholder =
        gtk::Label::new(Some("Dropped files and pasted images appear here"));
    attachments_placeholder.set_halign(gtk::Align::Start);
    attachments_placeholder.add_css_class("dim-label");

    let attachment_items = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let attachments = gtk::Box::new(gtk::Orientation::Vertical, 6);
    attachments.set_margin_top(6);
    attachments.set_margin_bottom(12);
    attachments.set_margin_start(12);
    attachments.set_margin_end(12);
    attachments.append(&attachments_title);
    attachments.append(&attachments_placeholder);
    attachments.append(&attachment_items);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&editor_scroll);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&attachments);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&toolbar));
    window.set_content(Some(&overlay));

    let initial_text = draft.text().to_owned();
    if !initial_text.is_empty() {
        buffer.set_text(&initial_text);
    }
    refresh_attachments(&attachment_items, &attachments_placeholder, &draft);

    let session = Rc::new(RefCell::new(draft));
    let debounce = Rc::new(RefCell::new(None::<glib::SourceId>));

    let changed_session = Rc::clone(&session);
    let changed_debounce = Rc::clone(&debounce);
    let changed_window = window.clone();
    let changed_title = title_widget.clone();
    let changed_overlay = overlay.clone();
    buffer.connect_changed(move |buffer| {
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string();
        changed_session.borrow_mut().set_text(text);
        changed_title.set_subtitle("Saving…");

        if let Some(source) = changed_debounce.borrow_mut().take() {
            source.remove();
        }

        let save_session = Rc::clone(&changed_session);
        let save_window = changed_window.clone();
        let save_title = changed_title.clone();
        let save_overlay = changed_overlay.clone();
        let next_debounce = Rc::clone(&changed_debounce);
        let source = glib::timeout_add_local_once(AUTOSAVE_DELAY, move || {
            next_debounce.borrow_mut().take();
            save_now(&save_session, &save_window, &save_title, &save_overlay);
        });
        *changed_debounce.borrow_mut() = Some(source);
    });

    let close_session = Rc::clone(&session);
    let close_debounce = Rc::clone(&debounce);
    let close_window = window.clone();
    let close_title = title_widget.clone();
    let close_overlay = overlay.clone();
    window.connect_close_request(move |_| {
        if let Some(source) = close_debounce.borrow_mut().take() {
            source.remove();
        }

        if flush_now(&close_session, &close_window, &close_title, &close_overlay) {
            glib::Propagation::Proceed
        } else {
            glib::Propagation::Stop
        }
    });

    let paste_session = Rc::clone(&session);
    let paste_window = window.clone();
    let paste_title = title_widget.clone();
    let paste_overlay = overlay.clone();
    let paste_items = attachment_items.clone();
    let paste_placeholder = attachments_placeholder.clone();
    let key_controller = gtk::EventControllerKey::new();
    let window_weak = window.downgrade();
    key_controller.connect_key_pressed(move |_, key, _, state| {
        if key == gdk::Key::Escape {
            if let Some(window) = window_weak.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        } else if is_paste_shortcut(key, state) {
            paste_from_clipboard(
                &paste_session,
                &paste_window,
                &paste_title,
                &paste_overlay,
                &paste_items,
                &paste_placeholder,
            )
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    editor.grab_focus();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardPaste {
    Default,
    Image,
}

fn classify_clipboard<'a>(mime_types: impl IntoIterator<Item = &'a str>) -> ClipboardPaste {
    let mut has_image = false;
    let mut has_text = false;

    for mime in mime_types {
        if mime.starts_with("image/") {
            has_image = true;
        } else if mime.starts_with("text/") {
            has_text = true;
        }
    }

    if has_image && !has_text {
        ClipboardPaste::Image
    } else {
        ClipboardPaste::Default
    }
}

fn is_paste_shortcut(key: gdk::Key, state: gdk::ModifierType) -> bool {
    (key == gdk::Key::v || key == gdk::Key::V)
        && state.contains(gdk::ModifierType::CONTROL_MASK)
        && !state.contains(gdk::ModifierType::SHIFT_MASK)
}

fn paste_from_clipboard(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    items: &gtk::Box,
    placeholder: &gtk::Label,
) -> glib::Propagation {
    let Some(display) = gdk::Display::default() else {
        return glib::Propagation::Proceed;
    };
    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    let mime_types = formats.mime_types();
    let paste = classify_clipboard(mime_types.iter().map(|mime| mime.as_str()));

    if paste == ClipboardPaste::Default {
        return glib::Propagation::Proceed;
    }

    title.set_subtitle("Pasting image…");

    let paste_session = Rc::clone(session);
    let paste_window = window.clone();
    let paste_title = title.clone();
    let paste_overlay = overlay.clone();
    let paste_items = items.clone();
    let paste_placeholder = placeholder.clone();
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

        let bytes = texture.save_to_png_bytes();
        match paste_session.borrow_mut().add_png(bytes.as_ref()) {
            Ok(_) => {
                {
                    let draft = paste_session.borrow();
                    refresh_attachments(&paste_items, &paste_placeholder, &draft);
                }
                save_now(&paste_session, &paste_window, &paste_title, &paste_overlay);
            }
            Err(_) => {
                tracing::error!("failed to store pasted image");
                paste_title.set_subtitle("Paste failed");
                show_toast(&paste_overlay, "Could not save the pasted image.");
            }
        }
    });

    glib::Propagation::Stop
}

fn refresh_attachments(items: &gtk::Box, placeholder: &gtk::Label, draft: &NoteDraft) {
    while let Some(child) = items.first_child() {
        items.remove(&child);
    }

    let attachments = draft.attachments();
    placeholder.set_visible(attachments.is_empty());

    for attachment in attachments {
        let label = gtk::Label::new(Some(&attachment.original_name));
        label.set_halign(gtk::Align::Start);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.add_css_class("dim-label");
        items.append(&label);
    }
}

fn show_toast(overlay: &adw::ToastOverlay, message: &str) {
    let toast = adw::Toast::new(message);
    toast.set_timeout(5);
    overlay.add_toast(toast);
}

fn save_now(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> bool {
    let id = session.borrow().id();
    let outcome = session.borrow_mut().save_if_needed();
    handle_save_outcome(outcome, id, window, title, overlay)
}

fn flush_now(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> bool {
    let id = session.borrow().id();
    let outcome = session.borrow_mut().finish();
    handle_save_outcome(outcome, id, window, title, overlay)
}

fn handle_save_outcome(
    outcome: Result<SaveOutcome, StorageError>,
    id: Uuid,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> bool {
    if matches!(outcome, Ok(SaveOutcome::Saved)) {
        window.set_widget_name(&existing_widget_name(id));
    }

    match outcome {
        Ok(SaveOutcome::Saved) => {
            title.set_subtitle("Saved");
            true
        }
        Ok(SaveOutcome::Deferred) => {
            title.set_subtitle("");
            true
        }
        Ok(SaveOutcome::Clean) => true,
        Err(_error) => {
            tracing::error!(note_id = %id, "note save failed");
            title.set_subtitle("Save failed");
            show_toast(overlay, SAVE_ERROR_MESSAGE);

            false
        }
    }
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
}
