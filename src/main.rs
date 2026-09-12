mod cli;
mod commands;
mod external_links;
mod format_command;
mod output;

use commands::{CommandOutcome, run_command};

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    let command = cli::parse_args(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(2);
    });

    match run_command(command) {
        Ok(CommandOutcome::Success) => {}
        Ok(CommandOutcome::CheckFailed) => std::process::exit(1),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
