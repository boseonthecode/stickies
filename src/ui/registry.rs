use std::cell::Cell;

use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use crate::storage::note::NoteStore;
use crate::ui::{browser_window, note_window};

pub struct WindowRegistry {
    next_note: Cell<u64>,
    store: NoteStore,
}

impl WindowRegistry {
    pub fn new(store: NoteStore) -> Self {
        Self {
            next_note: Cell::new(0),
            store,
        }
    }

    pub fn open_note(&self, app: &adw::Application) {
        let number = self.next_note.get() + 1;
        self.next_note.set(number);
        tracing::info!(note_number = number, "creating note window");
        note_window::present(app, number, self.store.clone());
    }

    pub fn open_note_id(&self, app: &adw::Application, id: Uuid) {
        let widget_name = note_window::existing_widget_name(id);

        if let Some(window) = app
            .windows()
            .iter()
            .find(|window| window.widget_name().as_str() == widget_name)
        {
            tracing::info!(note_id = %id, "focusing existing note window");
            window.present();
            return;
        }

        let number = self.next_note.get() + 1;
        self.next_note.set(number);
        tracing::info!(note_number = number, note_id = %id, "creating note window");
        if note_window::present_existing(app, number, self.store.clone(), id).is_err() {
            tracing::error!(note_id = %id, "failed to open note");
        }
    }

    pub fn open_browser(&self, app: &adw::Application) {
        if let Some(window) = app
            .windows()
            .iter()
            .find(|window| window.widget_name().as_str() == browser_window::WIDGET_NAME)
        {
            tracing::info!("focusing existing browser window");
            window.present();
            return;
        }

        tracing::info!("creating browser window");
        browser_window::present(app);
    }
}
