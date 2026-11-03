use clap::Parser;
use gio::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::cli::Cli;

const BROWSER_WINDOW: &str = "browser-window";
const NOTE_WINDOW: &str = "note-window";

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(crate::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(setup_actions);
    app.connect_activate(open_browser_window);
    app.connect_command_line(handle_command_line);
    app.run()
}

fn setup_actions(app: &adw::Application) {
    // Startup only runs in the primary instance; the hold keeps it resident after windows close.
    std::mem::forget(app.hold());
    tracing::info!("application startup");

    let new_action = gio::SimpleAction::new("new", None);
    let new_app = app.clone();
    new_action.connect_activate(move |_, _| open_note_window(&new_app));
    app.add_action(&new_action);

    let browse_action = gio::SimpleAction::new("browse", None);
    let browse_app = app.clone();
    browse_action.connect_activate(move |_, _| open_browser_window(&browse_app));
    app.add_action(&browse_action);

    let quit_action = gio::SimpleAction::new("quit", None);
    let quit_app = app.clone();
    quit_action.connect_activate(move |_, _| quit_app.quit());
    app.add_action(&quit_action);
}

fn handle_command_line(
    app: &adw::Application,
    command_line: &gio::ApplicationCommandLine,
) -> glib::ExitCode {
    let cli = match Cli::try_parse_from(command_line.arguments()) {
        Ok(cli) => cli,
        Err(error) => {
            command_line.printerr_literal(&error.to_string());
            return glib::ExitCode::FAILURE;
        }
    };

    match cli.action() {
        Some(action) => {
            tracing::info!(action = action.name(), "dispatching action");
            app.activate_action(action.name(), None);
        }
        None => tracing::info!("starting resident process"),
    }

    glib::ExitCode::SUCCESS
}

fn open_note_window(app: &adw::Application) {
    tracing::info!("creating note window");
    present_window(app, "Stickies Note", NOTE_WINDOW);
}

fn open_browser_window(app: &adw::Application) {
    if let Some(window) = app
        .windows()
        .iter()
        .find(|window| window.widget_name().as_str() == BROWSER_WINDOW)
    {
        tracing::info!("focusing existing browser window");
        window.present();
        return;
    }

    tracing::info!("creating browser window");
    present_window(app, "Stickies", BROWSER_WINDOW);
}

fn present_window(app: &adw::Application, title: &str, widget_name: &str) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(420)
        .default_height(320)
        .build();
    window.set_widget_name(widget_name);

    let header = adw::HeaderBar::new();
    let content = gtk::Label::new(Some(title));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    window.set_content(Some(&toolbar));
    window.present();
}
