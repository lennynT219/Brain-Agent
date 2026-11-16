use anyhow::{Context, Result, bail};
use dialoguer::{Confirm, Input};
use std::{
    fs::{create_dir_all, read_dir},
    path::{Path, PathBuf},
};

use crate::vault::BASE_DIRECTORIES;

const TOP_LEVEL_BASE_DIRECTORIES: &[&str] = &[
    "00-limbo",
    "01-proyectos",
    "02-areas",
    "03-recursos",
    "04-archivo",
    "99-sistema",
];

pub fn execute(path: Option<PathBuf>, dry_run: bool) -> Result<()> {
    let vault_path = resolve_vault_path(path)?;

    validate_vault_path(&vault_path)?;

    let has_existing_structure = contains_base_structure(&vault_path);
    let is_nonempty = vault_path.is_dir() && directory_isnoempty(&vault_path)?;

    if is_nonempty && !has_existing_structure && !dry_run {
        let confirmed = Confirm::new()
            .with_prompt(format!(
                "{} no esta vacío y no contiene una estructura de Brain. Quieres continuar?",
                vault_path.display()
            ))
            .default(false)
            .interact()
            .context("no se pudo leer la configuracion desde la terminal")?;

        if !confirmed {
            bail!("inicialización cancelada");
        }
    }

    let missing_directories = missing_directories(&vault_path);

    if dry_run {
        println!("Simunlación. Se crearían los siguientes directorios");
        for directory in &missing_directories {
            println!(" {}", directory.display());
        }
        return Ok(());
    }

    for directory in &missing_directories {
        create_dir_all(directory)
            .with_context(|| format!("no se pudo crear {}", directory.display()))?;
    }

    if missing_directories.is_empty() {
        println!(
            "La estructura de Brain ya existe en {}",
            vault_path.display()
        );
    } else {
        println!("Vault de Brain inicializando en {}", vault_path.display());
        for diretory in missing_directories {
            println!(" creando {}", diretory.display());
        }
    }

    Ok(())
}

fn resolve_vault_path(path: Option<PathBuf>) -> Result<PathBuf> {
    let path = match path {
        Some(path) => path,
        None => {
            let input: String = Input::new()
                .with_prompt("Ruta del vault")
                .interact_text()
                .context("no se pudo leer la ruta del vault desde la terminal")?;

            if input.trim().is_empty() {
                bail!("ruta del vault no puede estar vacía");
            }

            PathBuf::from(input.trim())
        }
    };

    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .context("no se pudo determinar el directorio actual")
            .map(|current_dir| current_dir.join(path))
    }
}

fn validate_vault_path(vault_path: &Path) -> Result<()> {
    if vault_path.exists() && !vault_path.is_dir() {
        bail!("{} existe, pero no es un directorio", vault_path.display());
    }

    for directory in BASE_DIRECTORIES {
        let expected_path = vault_path.join(directory);
        if expected_path.exists() && !expected_path.is_dir() {
            bail!(
                "{} existe, pero no es un directorio",
                expected_path.display()
            );
        }
    }

    Ok(())
}

fn contains_base_structure(vault_path: &Path) -> bool {
    TOP_LEVEL_BASE_DIRECTORIES
        .iter()
        .any(|directory| vault_path.join(directory).is_dir())
}

fn directory_isnoempty(path: &Path) -> Result<bool> {
    let mut entries =
        read_dir(path).with_context(|| format!("no se pudo inspeccionar {}", path.display()))?;

    Ok(entries.next().transpose()?.is_some())
}

fn missing_directories(vault_path: &Path) -> Vec<PathBuf> {
    BASE_DIRECTORIES
        .iter()
        .map(|directory| vault_path.join(directory))
        .filter(|path| !path.exists())
        .collect()
}
