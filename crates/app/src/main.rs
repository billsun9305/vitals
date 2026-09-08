mod cli;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let args = Cli::parse();
    let result = match args.command {
        Some(Command::Snapshot { interval, json: _, human }) => cli::run_snapshot(interval, human),
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
