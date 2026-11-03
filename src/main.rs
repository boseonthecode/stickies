use clap::Parser;
use stickies::cli::Cli;
use stickies::{application, logging};

fn main() -> glib::ExitCode {
    let cli = Cli::parse();
    logging::init();

    if cli.background || cli.gapplication_service {
        tracing::info!("starting resident process");
    }

    application::run()
}
