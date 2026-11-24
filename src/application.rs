use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::GtkApplicationExt;
use libadwaita as adw;

use crate::cli::Cli;
use crate::config::DataPaths;
use crate::storage::index::SearchIndex;
use crate::storage::note::NoteStore;
use crate::ui::registry::WindowRegistry;

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(crate::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // Startup only runs in the primary instance. Secondary invocations forward their
    // command line and must not open or rebuild the index on the way through.
    let startup_registry = Rc::new(RefCell::new(None::<Rc<WindowRegistry>>));
    let activate_registry = Rc::clone(&startup_registry);
    app.connect_startup(move |app| match initialize_registry() {
        Some(registry) => {
            *startup_registry.borrow_mut() = Some(Rc::clone(&registry));
            setup_actions(app, &registry);
        }
        None => app.quit(),
    });
    app.connect_activate(move |app| {
        if let Some(registry) = activate_registry.borrow().as_ref() {
            registry.open_browser(app);
        }
    });
    app.connect_command_line(handle_command_line);
    app.run()
}

fn initialize_registry() -> Option<Rc<WindowRegistry>> {
    let data_paths = match DataPaths::resolve().and_then(|paths| {
        paths.ensure_dirs()?;
        Ok(paths)
    }) {
        Ok(paths) => paths,
        Err(error) => {
            tracing::error!(%error, "failed to initialize data directory");
            return None;
        }
    };
    tracing::info!(root = %data_paths.root.display(), "data directories ready");

    let store = match SearchIndex::open(&data_paths) {
        Ok(index) => NoteStore::new(data_paths).with_index(Rc::new(index)),
        Err(error) => {
            tracing::error!(%error, "failed to open search index");
            NoteStore::new(data_paths)
        }
    };

    Some(Rc::new(WindowRegistry::new(store)))
}

fn setup_actions(app: &adw::Application, registry: &Rc<WindowRegistry>) {
    // Startup only runs in the primary instance; the hold keeps it resident after windows close.
    std::mem::forget(app.hold());
    tracing::info!("application startup");

    let new_action = gio::SimpleAction::new("new", None);
    let new_app = app.clone();
    let new_registry = Rc::clone(registry);
    new_action.connect_activate(move |_, _| new_registry.open_note(&new_app));
    app.add_action(&new_action);

    let browse_action = gio::SimpleAction::new("browse", None);
    let browse_app = app.clone();
    let browse_registry = Rc::clone(registry);
    browse_action.connect_activate(move |_, _| browse_registry.open_browser(&browse_app));
    app.add_action(&browse_action);

    let quit_action = gio::SimpleAction::new("quit", None);
    let quit_app = app.clone();
    quit_action.connect_activate(move |_, _| quit_app.quit());
    app.add_action(&quit_action);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
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
