use std::cell::Cell;

use libadwaita as adw;
use libadwaita::prelude::*;

use crate::ui::{browser_window, note_window};

#[derive(Default)]
pub struct WindowRegistry {
    next_note: Cell<u64>,
}

impl WindowRegistry {
    pub fn open_note(&self, app: &adw::Application) {
        let number = self.next_note.get() + 1;
        self.next_note.set(number);
        tracing::info!(note_number = number, "creating note window");
        note_window::present(app, number);
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
