mod cli;
mod commands;
mod output;

use commands::run_command;

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    let command = cli::parse_args(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(2);
    });

    run_command(command).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    })
}
