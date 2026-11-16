mod cli;
mod commands;
mod config;
mod vault;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() {
    if let Err(error) = run() {
        eprintln!("error:{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { path, dry_run } => commands::init::execute(path, dry_run),
        Command::Config { command } => commands::config::execute(command),
    }
}
