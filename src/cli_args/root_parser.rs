//! `clap::Parser` for [`Cli`], with the root flags in their own clap frame.
//!
//! In an unoptimised build, every builder temporary that the clap derive
//! emits keeps its own stack slot, so a derived `augment_*` function grows
//! with the number of arguments it adds inline. A `#[command(subcommand)]`
//! field is augmented from inside that same frame. When `Cli` derived
//! `Parser` directly, the frame that adds about 40 root flags (480 KiB) stayed
//! on the stack under the whole subcommand tree (#233).
//!
//! `CliParser` holds the subcommand and flattens `Cli`. The root flags are
//! then added by `<Cli as Args>::augment_args`, a call that is a sibling of
//! the subcommand-tree call, not around it. The command tree and the parse
//! result do not change.

use std::ffi::OsString;

use clap::{ArgMatches, Command, CommandFactory, Error, FromArgMatches, Parser};

use super::{Cli, Commands};

#[derive(Parser)]
#[command(name = "archon")]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("ARCHON_GIT_HASH"), ")"))]
#[command(about = "Archon CLI -- Rust-native AI agent runtime", long_about = None)]
struct CliParser {
    #[command(subcommand)]
    command: Option<Commands>,
    #[command(flatten)]
    cli: Cli,
}

impl CliParser {
    fn into_cli(self) -> Cli {
        let mut cli = self.cli;
        cli.command = self.command;
        cli
    }
}

impl CommandFactory for Cli {
    fn command() -> Command {
        CliParser::command()
    }
    fn command_for_update() -> Command {
        CliParser::command_for_update()
    }
}

impl Parser for Cli {
    fn parse() -> Self {
        CliParser::parse().into_cli()
    }
    fn try_parse() -> Result<Self, Error> {
        CliParser::try_parse().map(CliParser::into_cli)
    }
    fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        CliParser::parse_from(itr).into_cli()
    }
    fn try_parse_from<I, T>(itr: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        CliParser::try_parse_from(itr).map(CliParser::into_cli)
    }
    fn update_from<I, T>(&mut self, itr: I)
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        if let Err(error) = self.try_update_from(itr) {
            error.exit()
        }
    }
    fn try_update_from<I, T>(&mut self, itr: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut matches = Self::command_for_update().try_get_matches_from(itr)?;
        update_cli(self, &mut matches).map_err(|error| error.format(&mut Self::command()))
    }
}

/// The update that the derive generates for `CliParser`, applied to `Cli`.
fn update_cli(cli: &mut Cli, matches: &mut ArgMatches) -> Result<(), Error> {
    match cli.command.as_mut() {
        Some(command) => command.update_from_arg_matches_mut(matches)?,
        None => cli.command = Some(Commands::from_arg_matches_mut(matches)?),
    }
    <Cli as FromArgMatches>::update_from_arg_matches_mut(cli, matches)
}
