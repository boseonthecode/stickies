use clap::Parser;

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
    let cli = cli::Cli::parse();
    logging::init();

    if cli.background || cli.gapplication_service {
        tracing::info!("starting resident process");
    }

    application::run()
}
