use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gdk4 as gdk;
use gio;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use crate::storage::index::{NoteSummary, TrashFilter};
use crate::storage::note::{Note, NoteStore};
use crate::storage::trash;
use crate::ui::dialogs;
use crate::ui::note_card::{self, ThumbnailCache, summary_subtitle};
use crate::ui::registry::WindowRegistry;
use crate::util::text::note_title;

pub const WIDGET_NAME: &str = "browser-window";

const SEARCH_DELAY: Duration = Duration::from_millis(150);

struct BrowserState {
    app: adw::Application,
    registry: Rc<WindowRegistry>,
    store: NoteStore,
    window: glib::WeakRef<adw::ApplicationWindow>,
    list: gtk::ListView,
    note_store: gio::ListStore,
    list_selection: gtk::SingleSelection,
    grid: gtk::GridView,
    grid_selection: gtk::SingleSelection,
    view_stack: gtk::Stack,
    empty_page: adw::StatusPage,
    count_label: gtk::Label,
    search_entry: gtk::SearchEntry,
    search_debounce: RefCell<Option<glib::SourceId>>,
    trash_toggle: gtk::ToggleButton,
    toast_overlay: adw::ToastOverlay,
}

pub fn present(app: &adw::Application, registry: Rc<WindowRegistry>, store: NoteStore) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Stickies")
        .default_width(820)
        .default_height(560)
        .build();
    window.set_widget_name(WIDGET_NAME);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search notes")
        .width_chars(32)
        .build();
    search_entry.update_property(&[gtk::accessible::Property::Label("Search notes")]);
    search_entry.set_tooltip_text(Some(
        "Search notes; press Down for results, Escape to clear",
    ));

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&search_entry));

    let trash_toggle = gtk::ToggleButton::builder()
        .label("Trash")
        .tooltip_text("Show trashed notes")
        .build();
    trash_toggle.add_css_class("flat");
    trash_toggle.update_property(&[gtk::accessible::Property::Label("Show trashed notes")]);

    let note_store = gio::ListStore::new::<gtk::StringObject>();
    let list_selection = gtk::SingleSelection::new(Some(note_store.clone()));
    let list = gtk::ListView::builder()
        .model(&list_selection)
        .single_click_activate(false)
        .build();
    list.add_css_class("boxed-list");
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.update_property(&[gtk::accessible::Property::Label("Notes")]);

    let list_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();

    let empty_page = adw::StatusPage::builder()
        .icon_name("document-new-symbolic")
        .title("No notes yet")
        .description("Create a note to get started.")
        .build();

    let thumbnails = Rc::new(ThumbnailCache::default());
    let grid_selection = gtk::SingleSelection::new(Some(note_store.clone()));
    let grid = gtk::GridView::builder()
        .model(&grid_selection)
        .min_columns(2)
        .max_columns(6)
        .single_click_activate(true)
        .build();
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);
    grid.set_margin_start(12);
    grid.set_margin_end(12);
    grid.update_property(&[gtk::accessible::Property::Label("Note cards")]);

    let factory = gtk::SignalListItemFactory::new();
    let factory_store = store.clone();
    let factory_thumbnails = Rc::clone(&thumbnails);
    factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(object) = list_item.item() else {
            return;
        };
        let Some(string) = object.downcast_ref::<gtk::StringObject>() else {
            return;
        };
        let Ok(id) = Uuid::parse_str(string.string().as_str()) else {
            return;
        };
        match factory_store.load(id) {
            Ok(note) => {
                let card = note_card::build(&note, &factory_store, &factory_thumbnails);
                list_item.set_child(Some(&card));
            }
            Err(error) => {
                tracing::warn!(note_id = %id, %error, "skipping unreadable grid note");
                list_item.set_child(None::<&gtk::Widget>);
            }
        }
    });
    factory.connect_unbind(|_, list_item| {
        if let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() {
            list_item.set_child(None::<&gtk::Widget>);
        }
    });
    grid.set_factory(Some(&factory));

    let grid_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&grid)
        .build();

    let count_label = gtk::Label::new(None);
    count_label.set_xalign(0.0);
    count_label.add_css_class("dim-label");
    count_label.set_margin_top(8);
    count_label.set_margin_start(18);
    count_label.set_margin_end(18);

    let view_stack = gtk::Stack::new();
    view_stack.add_titled(&list_scroll, Some("list"), "List");
    view_stack.add_titled(&grid_scroll, Some("grid"), "Grid");
    view_stack.set_visible_child_name("list");

    empty_page.set_visible(false);
    let view_overlay = gtk::Overlay::new();
    view_overlay.set_child(Some(&view_stack));
    view_overlay.add_overlay(&empty_page);
    view_overlay.set_vexpand(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&count_label);
    content.append(&view_overlay);
    content.set_vexpand(true);

    let switcher = gtk::StackSwitcher::builder().stack(&view_stack).build();
    header.pack_end(&switcher);
    header.pack_end(&trash_toggle);

    let toast_overlay = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    toast_overlay.set_child(Some(&toolbar));

    window.set_content(Some(&toast_overlay));

    let state = Rc::new(BrowserState {
        app: app.clone(),
        registry,
        store,
        window: window.downgrade(),
        list,
        note_store,
        list_selection,
        grid,
        grid_selection,
        view_stack,
        empty_page,
        count_label,
        search_entry: search_entry.clone(),
        search_debounce: RefCell::new(None),
        trash_toggle: trash_toggle.clone(),
        toast_overlay: toast_overlay.clone(),
    });

    let list_factory = gtk::SignalListItemFactory::new();
    let list_bind_state = Rc::downgrade(&state);
    list_factory.connect_bind(move |_, list_item| {
        let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(state) = list_bind_state.upgrade() else {
            return;
        };
        let Some(id) = state
            .note_store
            .item(list_item.position())
            .and_then(|object| object.downcast_ref::<gtk::StringObject>().cloned())
            .and_then(|string| Uuid::parse_str(string.string().as_str()).ok())
        else {
            return;
        };
        match state.store.load(id) {
            Ok(note) => list_item.set_child(Some(&state.note_row(&NoteSummary::from_note(&note)))),
            Err(error) => {
                tracing::warn!(note_id = %id, %error, "skipping unreadable list note");
                list_item.set_child(None::<&gtk::Widget>);
            }
        }
    });
    list_factory.connect_unbind(|_, list_item| {
        if let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() {
            list_item.set_child(None::<&gtk::Widget>);
        }
    });
    state.list.set_factory(Some(&list_factory));

    let activated_state = Rc::downgrade(&state);
    state.list.connect_activate(move |_, position| {
        let Some(state) = activated_state.upgrade() else {
            return;
        };
        if let Some(id) = state.note_id_at(position) {
            state.open_note(id);
        }
    });

    let toggled_state = Rc::downgrade(&state);
    trash_toggle.connect_toggled(move |_| {
        if let Some(state) = toggled_state.upgrade() {
            state.refresh();
        }
    });

    let changed_state = Rc::downgrade(&state);
    search_entry.connect_search_changed(move |_| {
        let Some(state) = changed_state.upgrade() else {
            return;
        };
        if let Some(source) = state.search_debounce.borrow_mut().take() {
            source.remove();
        }
        let delayed_state = Rc::downgrade(&state);
        let source = glib::timeout_add_local_once(SEARCH_DELAY, move || {
            if let Some(state) = delayed_state.upgrade() {
                state.search_debounce.borrow_mut().take();
                state.refresh();
            }
        });
        *state.search_debounce.borrow_mut() = Some(source);
    });

    let activated_search = Rc::downgrade(&state);
    search_entry.connect_activate(move |_| {
        if let Some(state) = activated_search.upgrade() {
            state.open_first_result();
        }
    });

    let search_keys = gtk::EventControllerKey::new();
    let search_key_state = Rc::downgrade(&state);
    search_keys.connect_key_pressed(move |_, key, _, _| {
        let Some(state) = search_key_state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Down | gdk::Key::KP_Down => {
                state.focus_first_result();
                glib::Propagation::Stop
            }
            gdk::Key::Escape if !state.query().is_empty() => {
                state.search_entry.set_text("");
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    search_entry.add_controller(search_keys);

    let list_keys = gtk::EventControllerKey::new();
    let list_key_state = Rc::downgrade(&state);
    list_keys.connect_key_pressed(move |_, key, _, _| {
        let Some(state) = list_key_state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                state.delete_selected();
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                state.open_selected();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    state.list.add_controller(list_keys);

    let activated_grid = Rc::downgrade(&state);
    state.grid.connect_activate(move |_, position| {
        if let Some(state) = activated_grid.upgrade()
            && let Some(id) = state.note_id_at(position)
        {
            state.open_note(id);
        }
    });

    let grid_keys = gtk::EventControllerKey::new();
    let grid_key_state = Rc::downgrade(&state);
    grid_keys.connect_key_pressed(move |_, key, _, _| {
        if !matches!(key, gdk::Key::Delete | gdk::Key::KP_Delete) {
            return glib::Propagation::Proceed;
        }
        let Some(state) = grid_key_state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        state.delete_grid_selected();
        glib::Propagation::Stop
    });
    state.grid.add_controller(grid_keys);

    let focus_state = Rc::downgrade(&state);
    window.connect_notify_local(Some("is-active"), move |window, _| {
        if window.is_active()
            && let Some(state) = focus_state.upgrade()
        {
            state.refresh();
        }
    });

    let keep_alive = Rc::clone(&state);
    window.connect_close_request(move |_| {
        let _ = &keep_alive;
        tracing::info!("closing browser window");
        glib::Propagation::Proceed
    });

    state.refresh();
    window.present();
    search_entry.grab_focus();
    tracing::debug!("browser window presented");
}

impl BrowserState {
    fn refresh(self: &Rc<Self>) {
        let started = Instant::now();

        let query = self.query();
        let notes = self.load_summaries(&query);
        let loaded_at = started.elapsed();

        if notes.is_empty() {
            self.note_store
                .splice(0, self.note_store.n_items(), &[] as &[gtk::StringObject]);
            let showing_trash = self.showing_trash();
            if query.is_empty() {
                self.empty_page.set_icon_name(Some(if showing_trash {
                    "user-trash-symbolic"
                } else {
                    "document-new-symbolic"
                }));
                self.empty_page.set_title(if showing_trash {
                    "Trash is empty"
                } else {
                    "No notes yet"
                });
                self.empty_page.set_description(Some(if showing_trash {
                    "Notes you move to Trash will appear here."
                } else {
                    "Create a note to get started."
                }));
            } else {
                self.empty_page
                    .set_icon_name(Some("system-search-symbolic"));
                self.empty_page.set_title("No matches");
                self.empty_page
                    .set_description(Some(&format!("No notes match “{query}”.")));
            }
            self.count_label.set_visible(false);
            self.empty_page.set_visible(true);
            tracing::debug!(
                notes = 0,
                elapsed_ms = started.elapsed().as_millis(),
                "browser results refreshed"
            );
            return;
        }

        let objects = notes
            .iter()
            .map(|note| gtk::StringObject::new(&note.id.to_string()))
            .collect::<Vec<_>>();
        let built_at = started.elapsed();
        self.note_store
            .splice(0, self.note_store.n_items(), &objects);
        let spliced_at = started.elapsed();
        self.count_label.set_text(&result_count_label(
            notes.len(),
            !query.is_empty(),
            self.showing_trash(),
        ));
        self.count_label.set_visible(true);
        self.empty_page.set_visible(false);
        self.view_stack.set_visible(true);
        self.list_selection.set_selected(0);
        self.grid_selection.set_selected(0);
        tracing::debug!(
            notes = notes.len(),
            elapsed_ms = started.elapsed().as_millis(),
            load_ms = loaded_at.as_millis(),
            build_ms = (built_at - loaded_at).as_millis(),
            splice_ms = (spliced_at - built_at).as_millis(),
            select_ms = (started.elapsed() - spliced_at).as_millis(),
            "browser results refreshed"
        );
    }

    fn load_summaries(&self, query: &str) -> Vec<NoteSummary> {
        if let Some(index) = self.store.index() {
            match index.search_summaries(query, self.filter()) {
                Ok(summaries) => return summaries,
                Err(error) => {
                    tracing::warn!(%error, "search index unavailable for browser listing");
                }
            }
        }

        let ids = match self.store.list_ids() {
            Ok(ids) => ids,
            Err(error) => {
                tracing::error!(%error, "failed to list notes");
                self.toast("Could not load notes.");
                return Vec::new();
            }
        };

        let mut summaries = ids
            .into_iter()
            .filter_map(|id| match self.store.load(id) {
                Ok(note) => Some(note),
                Err(error) => {
                    tracing::warn!(note_id = %id, %error, "skipping unreadable browser note");
                    None
                }
            })
            .filter(|note| match self.filter() {
                TrashFilter::Exclude => !note.meta.trashed,
                TrashFilter::Only => note.meta.trashed,
                TrashFilter::Include => true,
            })
            .filter(|note| query.is_empty() || note_matches(note, query))
            .map(|note| NoteSummary::from_note(&note))
            .collect::<Vec<_>>();

        summaries.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.modified_at.cmp(&left.modified_at))
        });
        summaries
    }

    fn note_row(self: &Rc<Self>, note: &NoteSummary) -> adw::ActionRow {
        let title = note.title.clone();
        let subtitle = summary_subtitle(note);
        let action_row = adw::ActionRow::builder()
            .title(title.clone())
            .subtitle(subtitle)
            .build();
        action_row.set_focusable(false);

        let indicators = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        indicators.set_valign(gtk::Align::Center);
        if note.pinned {
            let icon = gtk::Image::from_icon_name("view-pin-symbolic");
            icon.set_tooltip_text(Some("Pinned"));
            indicators.append(&icon);
        }
        if note.has_attachments {
            let icon = gtk::Image::from_icon_name("mail-attachment-symbolic");
            icon.set_tooltip_text(Some("Has attachments"));
            indicators.append(&icon);
        }
        if indicators.first_child().is_some() {
            action_row.add_prefix(&indicators);
        }

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.set_valign(gtk::Align::Center);

        if note.trashed {
            let restore = icon_button("edit-undo-symbolic", "Restore note");
            let restore_state = Rc::downgrade(self);
            let restore_id = note.id;
            restore.connect_clicked(move |_| {
                if let Some(state) = restore_state.upgrade() {
                    state.restore_note(restore_id);
                }
            });
            actions.append(&restore);

            let delete = icon_button("user-trash-symbolic", "Delete permanently (Delete)");
            delete.add_css_class("destructive-action");
            let delete_state = Rc::downgrade(self);
            let delete_id = note.id;
            let delete_title = title;
            delete.connect_clicked(move |_| {
                if let Some(state) = delete_state.upgrade() {
                    state.confirm_purge(delete_id, &delete_title);
                }
            });
            actions.append(&delete);
        } else {
            let trash_button = icon_button("user-trash-symbolic", "Move to Trash (Delete)");
            let trash_state = Rc::downgrade(self);
            let trash_id = note.id;
            let trash_title = title;
            trash_button.connect_clicked(move |_| {
                if let Some(state) = trash_state.upgrade() {
                    state.confirm_trash(trash_id, &trash_title);
                }
            });
            actions.append(&trash_button);
        }

        if actions.first_child().is_some() {
            action_row.add_suffix(&actions);
        }

        action_row
    }

    fn open_note(&self, id: Uuid) {
        self.registry.open_note_id(&self.app, id);
    }

    fn open_selected(&self) {
        if let Some(id) = self.selected_note_id() {
            self.open_note(id);
        }
    }

    fn open_first_result(self: &Rc<Self>) {
        self.apply_pending_search();
        if self.grid_view_active() {
            if let Some(id) = self.note_id_at(0) {
                self.open_note(id);
            }
            return;
        }

        if let Some(id) = self.note_id_at(0) {
            self.open_note(id);
        }
    }

    fn focus_first_result(self: &Rc<Self>) {
        self.apply_pending_search();
        if self.grid_view_active() {
            self.grid_selection.set_selected(0);
            self.grid.grab_focus();
            return;
        }

        self.list_selection.set_selected(0);
        self.list.grab_focus();
    }

    fn apply_pending_search(self: &Rc<Self>) {
        if let Some(source) = self.search_debounce.borrow_mut().take() {
            source.remove();
        }
        self.refresh();
    }

    fn selected_note_id(&self) -> Option<Uuid> {
        self.note_id_at(self.list_selection.selected())
    }

    fn note_id_at(&self, position: u32) -> Option<Uuid> {
        let object = self.note_store.item(position)?;
        let string = object.downcast_ref::<gtk::StringObject>()?;
        Uuid::parse_str(string.string().as_str()).ok()
    }

    fn delete_selected(self: &Rc<Self>) {
        if let Some(id) = self.selected_note_id() {
            self.delete_note(id);
        }
    }

    fn delete_grid_selected(self: &Rc<Self>) {
        if let Some(id) = self.note_id_at(self.grid_selection.selected()) {
            self.delete_note(id);
        }
    }

    fn delete_note(self: &Rc<Self>, id: Uuid) {
        let title = self
            .store
            .load(id)
            .map(|note| note_title(&note.body))
            .unwrap_or_else(|_| "This note".to_owned());
        if self.showing_trash() {
            self.confirm_purge(id, &title);
        } else {
            self.confirm_trash(id, &title);
        }
    }

    fn confirm_trash(self: &Rc<Self>, id: Uuid, title: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = Rc::clone(self);
        dialogs::confirm_trash(&window, title, move || {
            state.trash_note(id);
        });
    }

    fn trash_note(self: &Rc<Self>, id: Uuid) {
        match trash::trash(&self.store, id) {
            Ok(_) => {
                self.toast("Note moved to Trash");
                self.refresh();
            }
            Err(error) => {
                tracing::error!(note_id = %id, %error, "failed to move note to Trash");
                self.toast("Could not move note to Trash.");
            }
        }
    }

    fn restore_note(self: &Rc<Self>, id: Uuid) {
        match trash::restore(&self.store, id) {
            Ok(_) => {
                self.toast("Note restored");
                self.refresh();
            }
            Err(error) => {
                tracing::error!(note_id = %id, %error, "failed to restore note");
                self.toast("Could not restore note.");
            }
        }
    }

    fn confirm_purge(self: &Rc<Self>, id: Uuid, title: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = Rc::clone(self);
        dialogs::confirm_purge(&window, title, move || {
            state.purge_note(id);
        });
    }

    fn purge_note(self: &Rc<Self>, id: Uuid) {
        match trash::purge(&self.store, id) {
            Ok(()) => {
                self.toast("Note permanently deleted");
                self.refresh();
            }
            Err(error) => {
                tracing::error!(note_id = %id, %error, "failed to purge note");
                self.toast("Could not delete note permanently.");
            }
        }
    }

    fn grid_view_active(&self) -> bool {
        self.view_stack.visible_child_name().as_deref() == Some("grid")
    }

    fn query(&self) -> String {
        self.search_entry.text().trim().to_owned()
    }

    fn showing_trash(&self) -> bool {
        self.trash_toggle.is_active()
    }

    fn filter(&self) -> TrashFilter {
        if self.showing_trash() {
            TrashFilter::Only
        } else {
            TrashFilter::Exclude
        }
    }

    fn toast(&self, message: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(message));
    }
}

fn icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon_name)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

fn note_matches(note: &Note, query: &str) -> bool {
    let query = query.to_lowercase();
    let bare_query = query.trim_start_matches('#').to_owned();
    let body = note.body.to_lowercase();
    let tags = note.meta.tags.join(" ").to_lowercase();
    let attachments = note
        .meta
        .attachments
        .iter()
        .map(|attachment| attachment.original_name.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    body.contains(&query)
        || tags.contains(&query)
        || tags.contains(&bare_query)
        || attachments.contains(&query)
}

fn result_count_label(count: usize, searching: bool, showing_trash: bool) -> String {
    match (count, searching, showing_trash) {
        (1, false, false) => "1 note".to_owned(),
        (count, false, false) => format!("{count} notes"),
        (1, false, true) => "1 note in Trash".to_owned(),
        (count, false, true) => format!("{count} notes in Trash"),
        (1, true, _) => "1 match".to_owned(),
        (count, true, _) => format!("{count} matches"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::storage::note::{AttachmentKind, AttachmentMeta, Note};

    #[test]
    fn matches_query_against_body_tags_and_attachment_names() {
        let mut note = Note::new();
        note.body = "Project Alpha".to_owned();
        note.meta.tags = vec!["Work".to_owned()];
        note.meta.attachments.push(AttachmentMeta {
            id: Uuid::now_v7(),
            stored_name: "stored.pdf".to_owned(),
            original_name: "Design-Brief.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: 10,
            kind: AttachmentKind::File,
            created_at: Utc::now(),
        });

        assert!(note_matches(&note, "alpha"));
        assert!(note_matches(&note, "WORK"));
        assert!(note_matches(&note, "#work"));
        assert!(note_matches(&note, "design-brief"));
        assert!(!note_matches(&note, "missing"));
    }

    #[test]
    fn formats_result_counts_for_search_and_trash() {
        assert_eq!(result_count_label(1, false, false), "1 note");
        assert_eq!(result_count_label(3, false, false), "3 notes");
        assert_eq!(result_count_label(1, false, true), "1 note in Trash");
        assert_eq!(result_count_label(3, false, true), "3 notes in Trash");
        assert_eq!(result_count_label(1, true, false), "1 match");
        assert_eq!(result_count_label(3, true, true), "3 matches");
    }
}
