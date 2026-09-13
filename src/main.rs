mod check_command;
mod cli;
mod commands;
mod external_links;
mod format_command;
mod output;

use commands::{CommandOutcome, run_command};

fn main() {
    let args = std::env::args_os()
        .map(|argument| argument.into_string())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|_| {
            eprintln!("command-line arguments must be valid UTF-8");
            std::process::exit(2);
        });

    let command = cli::parse_args(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(2);
    });

    match run_command(command) {
        Ok(CommandOutcome::Success) => {}
        Ok(CommandOutcome::CheckFailed) => std::process::exit(1),
        Ok(CommandOutcome::OperationalFailure) => std::process::exit(2),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(error.exit_code());
        }
    }
}
