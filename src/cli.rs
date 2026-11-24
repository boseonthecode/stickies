use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "stickies",
    version,
    about = "Keyboard-first local sticky notes"
)]
pub struct Cli {
    #[arg(long, hide = true)]
    pub background: bool,

    #[arg(long = "gapplication-service", hide = true)]
    pub gapplication_service: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum Command {
    New,
    Browse,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    New,
    Browse,
    Quit,
}

impl Cli {
    pub fn action(&self) -> Option<Action> {
        if self.background || self.gapplication_service {
            return None;
        }

        Some(match self.command.unwrap_or(Command::Browse) {
            Command::New => Action::New,
            Command::Browse => Action::Browse,
            Command::Quit => Action::Quit,
        })
    }
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Browse => "browse",
            Self::Quit => "quit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("valid cli arguments")
    }

    #[test]
    fn defaults_to_browse() {
        assert_eq!(parse(&["stickies"]).action(), Some(Action::Browse));
    }

    #[test]
    fn parses_new_command() {
        assert_eq!(parse(&["stickies", "new"]).action(), Some(Action::New));
    }

    #[test]
    fn parses_browse_command() {
        assert_eq!(
            parse(&["stickies", "browse"]).action(),
            Some(Action::Browse)
        );
    }

    #[test]
    fn parses_quit_command() {
        assert_eq!(parse(&["stickies", "quit"]).action(), Some(Action::Quit));
    }

    #[test]
    fn background_starts_without_action() {
        assert_eq!(parse(&["stickies", "--background"]).action(), None);
    }

    #[test]
    fn gapplication_service_starts_without_action() {
        assert_eq!(
            parse(&["stickies", "--gapplication-service"]).action(),
            None
        );
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(Cli::try_parse_from(["stickies", "unknown"]).is_err());
    }
}
