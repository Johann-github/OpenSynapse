//! CLI dispatch. See README.md.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "opensynapse", version, about = "Local-first Razer device configuration")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List connected Razer devices.
    List,
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::List => {
            println!("not implemented yet");
            Ok(())
        }
    }
}
