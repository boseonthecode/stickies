use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

mod application;
mod cli;
mod config;
mod error;
mod logging;
mod storage;
mod ui;
mod util;

const APP_ID: &str = "io.github.chthonic.Stickies";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_activate(build_window);
    app.run()
}

fn build_window(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Stickies")
        .default_width(420)
        .default_height(320)
        .build();

    let header = adw::HeaderBar::new();
    let content = gtk::Label::new(Some("Stickies"));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    window.set_content(Some(&toolbar));
    window.present();
}
