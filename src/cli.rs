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
