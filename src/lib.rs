pub mod document;
pub mod memory;

mod cli;
mod commands;
mod config;
mod notes;
mod project_binding;
mod projects;
mod vault;

/// Existing interactive commands and the opt-in local JSON adapter.
pub fn run_cli() -> anyhow::Result<()> {
    use clap::Parser;
    use cli::{Cli, Command};
    match Cli::parse().command {
        Command::Init { path, dry_run } => commands::init::execute(path, dry_run),
        Command::Config { command } => commands::config::execute(command),
        Command::Notes => commands::notes::execute(),
        Command::Projects => commands::projects::execute(),
        Command::Memory {
            max_input_bytes,
            max_output_bytes,
        } => commands::memory::execute(max_input_bytes, max_output_bytes),
    }
}
