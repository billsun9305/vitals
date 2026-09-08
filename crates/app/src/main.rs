mod cli;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let args = Cli::parse();
    let result = match args.command {
        Some(Command::Snapshot { interval, json: _, human }) => cli::run_snapshot(interval, human),
        Some(Command::Top { n, json: _, human }) => cli::run_top(n, human),
        Some(Command::Pressure { interval, json: _, human, exit_code }) =>
            cli::run_pressure(interval, human, exit_code),
        Some(Command::Watch { interval_s, count }) => cli::run_watch(interval_s, count),
        None => {
            eprintln!("vitals: the menu bar app is not built yet; try `vitals snapshot`");
            std::process::exit(1);
        }
    };
    if let Err(e) = result {
        eprintln!("vitals: {e}");
        std::process::exit(1);
    }
}
