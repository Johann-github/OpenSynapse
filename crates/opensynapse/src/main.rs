use clap::Parser as _;

fn main() -> anyhow::Result<()> {
    opensynapse_cli::run(opensynapse_cli::Cli::parse())
}
