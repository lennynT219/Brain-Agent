use std::{
    fs,
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use dialoguer::Confirm;

use crate::{
    cli::ConfigCommand,
    config::{CONFIG_VERSION, Config, config_file_path, load_config, save_config},
};

pub fn execute(command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => {
            let config = load_config()?;
            println!(
                "Archivo de configuración: {}",
                config_file_path()?.display()
            );
            println!("Versión: {}", config.config_version);
            println!("Vault: {}", config.vault_path.display());
            Ok(())
        }
        ConfigCommand::SetVault { path, yes } => set_vault(path, yes),
    }
}

fn set_vault(path: PathBuf, yes: bool) -> Result<()> {
    let vault_path = path
        .canonicalize()
        .with_context(|| format!("no se pudo resolver el vault {}", path.display()))?;
    let config = Config {
        config_version: CONFIG_VERSION,
        vault_path,
    };
    config.validate().context("vault seleccionado inválido")?;
    let destination = config_file_path()?;
    let exists = match fs::symlink_metadata(&destination) {
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "el destino no es un archivo regular: {}",
                destination.display()
            );
        }
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("no se pudo inspeccionar {}", destination.display()));
        }
    };
    if exists {
        if load_config().is_ok_and(|old| old.vault_path == config.vault_path) {
            println!(
                "El vault ya está seleccionado: {} (configuración sin cambios: {})",
                config.vault_path.display(),
                destination.display()
            );
            return Ok(());
        }
        if !yes {
            if !io::stdin().is_terminal() {
                bail!(
                    "se requiere confirmación para reemplazar {}; usa --yes sin terminal",
                    destination.display()
                );
            }
            let confirmed = Confirm::new()
                .with_prompt(format!("¿Reemplazar el contenido del archivo de configuración {} para seleccionar el vault {}?",
                    destination.display(), config.vault_path.display()))
                .default(false)
                .interact()
                .context("no se pudo leer la confirmación desde la terminal")?;
            if !confirmed {
                bail!("selección del vault cancelada; configuración sin cambios");
            }
        }
    }
    let saved = save_config(&config, exists)?;
    println!("Vault seleccionado: {}", config.vault_path.display());
    println!("Archivo de configuración: {}", saved.display());
    Ok(())
}
