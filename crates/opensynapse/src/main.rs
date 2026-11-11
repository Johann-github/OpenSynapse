use std::process::ExitCode;

use clap::Parser as _;

fn main() -> ExitCode {
    match opensynapse_cli::run(opensynapse_cli::Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Same output as returning the error from `main`, plus the optional
            // wake hint (off unless `opensynapse settings set wake-hint on`).
            eprintln!("Error: {e:?}");
            if let Some(hint) = opensynapse_cli::wake_hint_for(&e) {
                eprintln!("Hint: {hint}");
            }
            ExitCode::FAILURE
        }
    }
}
