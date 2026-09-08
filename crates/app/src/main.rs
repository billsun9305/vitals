use clap::Parser;
use vitals::cli::{self, Cli, Command};
use vitals::{serve, tray};

fn main() {
    let args = Cli::parse();
    let result = match args.command {
        Some(Command::Snapshot {
            interval,
            json: _,
            human,
        }) => cli::run_snapshot(interval, human),
        Some(Command::Top { n, json: _, human }) => cli::run_top(n, human),
        Some(Command::Pressure {
            interval,
            json: _,
            human,
            exit_code,
        }) => cli::run_pressure(interval, human, exit_code),
        Some(Command::Watch { interval_s, count }) => cli::run_watch(interval_s, count),
        Some(Command::Serve { port, no_open }) => serve::run(port, !no_open),
        Some(Command::Dashboard { port }) => serve::run(port, true),
        None => tray::run(),
    };
    if let Err(e) = result {
        eprintln!("vitals: {e}");
        std::process::exit(1);
    }
}
