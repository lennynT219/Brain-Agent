use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "brain",
    version,
    about = "Un agente de terminal para mantener un segundo cerebro en Obsidian",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// One bounded versioned JSON request on stdin; no interactive prompts.
    Memory {
        /// Transport bound, not a desired Memory size or retention policy.
        #[arg(long, default_value_t = 4 * 1024 * 1024, value_parser = clap::value_parser!(u64).range(1024..=1073741824))]
        max_input_bytes: u64,
        /// Output is rejected explicitly rather than truncated.
        #[arg(long, default_value_t = 4 * 1024 * 1024, value_parser = clap::value_parser!(u64).range(1024..=1073741824))]
        max_output_bytes: u64,
    },
    /// One bounded JSON request for the Project Notes bound to the current directory.
    Notes,
    ///Crear la estructura base PARA para un vault en Obsidian
    Init {
        ///Directorio donde se creara el vault
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Muestra los cambios previstos sin escribir en el sistema de archivos.
        #[arg(long)]
        dry_run: bool,
    },
    /// Selecciona o consulta el vault activo.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Selecciona un vault existente con estructura completa de Brain.
    SetVault {
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Autoriza reemplazar el contenido de la configuración existente.
        #[arg(long)]
        yes: bool,
    },
    /// Muestra la configuración validada sin modificarla.
    Show,
}
