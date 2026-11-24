use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gdk4 as gdk;
use gdk4::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use crate::error::StorageError;
use crate::storage::attachments;
use crate::storage::draft::{NoteDraft, SaveOutcome};
use crate::storage::note::NoteStore;
use crate::ui::attachment_view::AttachmentView;
use crate::ui::dialogs;
use crate::ui::editor::Editor;
use crate::ui::note_input::{
    attach_texture, finish_file_paste, is_paste_shortcut, local_file_paths, paste_from_clipboard,
    refresh_attachment_view,
};
use crate::ui::tag_view::TagView;

pub const WIDGET_NAME: &str = "note-window";

const AUTOSAVE_DELAY: Duration = Duration::from_millis(650);
const SAVE_ERROR_MESSAGE: &str = "Could not save note. Check disk space and permissions.";
const SAVE_FAILED_SUBTITLE: &str = "Save failed";

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

    let pin_button = gtk::ToggleButton::builder()
        .icon_name("view-pin-symbolic")
        .tooltip_text("Pin note")
        .build();
    pin_button.add_css_class("flat");
    pin_button.set_active(draft.pinned());
    pin_button.update_property(&[gtk::accessible::Property::Label("Pin note")]);
    header.pack_end(&pin_button);

    let editor = Editor::new();
    let buffer = editor.buffer();
    let source_view = editor.view();

    let editor_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&source_view)
        .vexpand(true)
        .build();

    let attachment_view = Rc::new(AttachmentView::new());
    let tag_view = TagView::new();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&editor_scroll);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&tag_view.widget());
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(attachment_view.widget());

    let drop_banner = adw::Banner::new("Drop to attach files or insert text");
    drop_banner.set_revealed(false);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&drop_banner);
    toolbar.set_content(Some(&content));

    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&toolbar));
    window.set_content(Some(&overlay));

    let initial_text = draft.text().to_owned();
    if !initial_text.is_empty() {
        buffer.set_text(&initial_text);
    }
    refresh_attachment_view(&attachment_view, &draft);

    let session = Rc::new(RefCell::new(draft));
    let debounce = Rc::new(RefCell::new(None::<glib::SourceId>));
    let closing = Rc::new(Cell::new(false));

    let add_session = Rc::clone(&session);
    let add_window = window.clone();
    let add_title = title_widget.clone();
    let add_overlay = overlay.clone();
    tag_view.set_add_handler(move |tags| {
        if add_session.borrow_mut().add_tags(&tags) {
            add_title.set_subtitle("Saving…");
            save_now(&add_session, &add_window, &add_title, &add_overlay);
        }
    });

    let remove_session = Rc::clone(&session);
    let remove_window = window.clone();
    let remove_title = title_widget.clone();
    let remove_overlay = overlay.clone();
    tag_view.set_remove_handler(move |tag| {
        if remove_session.borrow_mut().remove_tag(&tag) {
            remove_title.set_subtitle("Saving…");
            save_now(
                &remove_session,
                &remove_window,
                &remove_title,
                &remove_overlay,
            );
        }
    });
    tag_view.set_tags(session.borrow().tags());

    let pin_session = Rc::clone(&session);
    let pin_window = window.clone();
    let pin_title = title_widget.clone();
    let pin_overlay = overlay.clone();
    pin_button.connect_toggled(move |button| {
        if pin_session.borrow_mut().set_pinned(button.is_active()) {
            pin_title.set_subtitle("Saving…");
            save_now(&pin_session, &pin_window, &pin_title, &pin_overlay);
        }
    });

    let open_overlay = overlay.clone();
    attachment_view.set_open_handler(move |entry| {
        if entry.missing {
            show_toast(&open_overlay, "Attachment file is missing.");
            return;
        }

        let Some(path) = entry.path.as_ref() else {
            show_toast(&open_overlay, "Attachment file is missing.");
            return;
        };
        let file = gio::File::for_path(path);
        if gio::AppInfo::launch_default_for_uri(file.uri().as_str(), None::<&gio::AppLaunchContext>)
            .is_err()
        {
            tracing::error!("failed to open attachment");
            show_toast(&open_overlay, "Could not open the attachment.");
        }
    });

    let remove_session = Rc::clone(&session);
    let remove_window = window.clone();
    let remove_title = title_widget.clone();
    let remove_overlay = overlay.clone();
    let remove_view = Rc::clone(&attachment_view);
    attachment_view.set_remove_handler(move |attachment| {
        let dialog_session = Rc::clone(&remove_session);
        let dialog_window = remove_window.clone();
        let dialog_title = remove_title.clone();
        let dialog_overlay = remove_overlay.clone();
        let dialog_view = Rc::clone(&remove_view);
        let attachment_id = attachment.id;

        dialogs::confirm_remove_attachment(&remove_window, &attachment.original_name, move || {
            let removed = dialog_session.borrow_mut().remove_attachment(attachment_id);
            match removed {
                Ok(true) => {
                    {
                        let draft = dialog_session.borrow();
                        refresh_attachment_view(&dialog_view, &draft);
                    }
                    save_now(
                        &dialog_session,
                        &dialog_window,
                        &dialog_title,
                        &dialog_overlay,
                    );
                }
                Ok(false) => {}
                Err(_) => {
                    tracing::error!("failed to remove attachment");
                    show_toast(&dialog_overlay, "Could not remove the attachment.");
                }
            }
        });
    });

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
    let close_closing = Rc::clone(&closing);
    window.connect_close_request(move |_| {
        let note_id = close_session.borrow().id();
        tracing::info!(note_id = %note_id, "closing note window");
        close_closing.set(true);
        if let Some(source) = close_debounce.borrow_mut().take() {
            source.remove();
        }

        if flush_now(&close_session, &close_window, &close_title, &close_overlay) {
            glib::Propagation::Proceed
        } else {
            close_closing.set(false);
            glib::Propagation::Stop
        }
    });

    let paste_session = Rc::clone(&session);
    let paste_window = window.clone();
    let paste_title = title_widget.clone();
    let paste_overlay = overlay.clone();
    let paste_view = Rc::clone(&attachment_view);
    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
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
                &paste_view,
            )
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    let drop_session = Rc::clone(&session);
    let drop_window = window.clone();
    let drop_title = title_widget.clone();
    let drop_overlay = overlay.clone();
    let drop_view = Rc::clone(&attachment_view);
    let drop_buffer = buffer.clone();
    let drop_closing = Rc::clone(&closing);
    let drop_banner = drop_banner.clone();

    let enter_title = title_widget.clone();
    let enter_closing = Rc::clone(&closing);
    let enter_banner = drop_banner.clone();

    let leave_banner = drop_banner.clone();

    let drop_target = gtk::DropTarget::new(glib::types::Type::INVALID, gdk::DragAction::COPY);
    drop_target.set_types(&[
        gdk::FileList::static_type(),
        gdk::Texture::static_type(),
        glib::types::Type::STRING,
    ]);
    drop_target.connect_enter(move |_, _, _| {
        if enter_closing.get() || save_failed(&enter_title) {
            return gdk::DragAction::empty();
        }
        enter_banner.set_revealed(true);
        gdk::DragAction::COPY
    });
    drop_target.connect_leave(move |_| {
        leave_banner.set_revealed(false);
    });
    drop_target.connect_drop(move |_, value, _, _| {
        drop_banner.set_revealed(false);

        if drop_closing.get() || save_failed(&drop_title) {
            return true;
        }

        if let Ok(file_list) = value.get::<gdk::FileList>() {
            let (paths, rejected) = local_file_paths(&file_list);
            let mut inspection = attachments::inspect_files(paths);
            inspection.rejected.extend(rejected);
            finish_file_paste(
                &drop_session,
                &drop_window,
                &drop_title,
                &drop_overlay,
                &drop_view,
                inspection,
            );
            true
        } else if let Ok(texture) = value.get::<gdk::Texture>() {
            attach_texture(
                &texture,
                &drop_session,
                &drop_window,
                &drop_title,
                &drop_overlay,
                &drop_view,
            );
            true
        } else if let Ok(text) = value.get::<String>() {
            if text.is_empty() {
                return false;
            }
            drop_buffer.insert_at_cursor(&text);
            true
        } else {
            false
        }
    });
    source_view.add_controller(drop_target);
    window.present();
    tracing::debug!("note window presented");
    source_view.grab_focus();
}

pub(super) fn save_failed(title: &adw::WindowTitle) -> bool {
    title.subtitle().as_str() == SAVE_FAILED_SUBTITLE
}

pub(super) fn show_toast(overlay: &adw::ToastOverlay, message: &str) {
    let toast = adw::Toast::new(message);
    toast.set_timeout(5);
    overlay.add_toast(toast);
}

pub(super) fn save_now(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> bool {
    let id = session.borrow().id();
    let outcome = session.borrow_mut().save_if_needed();
    let retry = save_retry(session, window, title, overlay);
    handle_save_outcome(outcome, id, window, title, overlay, Some(retry))
}

fn flush_now(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> bool {
    let id = session.borrow().id();
    let outcome = session.borrow_mut().finish();
    let retry = save_retry(session, window, title, overlay);
    handle_save_outcome(outcome, id, window, title, overlay, Some(retry))
}

fn save_retry(
    session: &Rc<RefCell<NoteDraft>>,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
) -> Rc<dyn Fn()> {
    let session = Rc::clone(session);
    let window = window.clone();
    let title = title.clone();
    let overlay = overlay.clone();
    Rc::new(move || {
        save_now(&session, &window, &title, &overlay);
    })
}

fn handle_save_outcome(
    outcome: Result<SaveOutcome, StorageError>,
    id: Uuid,
    window: &adw::ApplicationWindow,
    title: &adw::WindowTitle,
    overlay: &adw::ToastOverlay,
    retry: Option<Rc<dyn Fn()>>,
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
            title.set_subtitle(SAVE_FAILED_SUBTITLE);

            let toast = adw::Toast::new(SAVE_ERROR_MESSAGE);
            toast.set_timeout(0);
            if let Some(retry) = retry {
                toast.set_button_label(Some("Retry"));
                toast.connect_button_clicked(move |_| retry());
            }
            overlay.add_toast(toast);

            false
        }
    }
}
