use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gdk4 as gdk;
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

    let attachments = gtk::Box::new(gtk::Orientation::Vertical, 6);
    attachments.set_margin_top(6);
    attachments.set_margin_bottom(12);
    attachments.set_margin_start(12);
    attachments.set_margin_end(12);
    attachments.append(&attachments_title);
    attachments.append(&attachments_placeholder);

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

    let key_controller = gtk::EventControllerKey::new();
    let window_weak = window.downgrade();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(window) = window_weak.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    editor.grab_focus();
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

            let toast = adw::Toast::new(SAVE_ERROR_MESSAGE);
            toast.set_timeout(5);
            overlay.add_toast(toast);

            false
        }
    }
}
