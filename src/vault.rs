use std::{fs, path::Path};

use anyhow::{Context, Result, bail};

pub(crate) const BASE_DIRECTORIES: &[&str] = &[
    "00-limbo",
    "01-proyectos",
    "02-areas",
    "03-recursos",
    "04-archivo",
    "99-sistema",
    "99-sistema/plantillas",
    "99-sistema/documentacion",
];

pub(crate) fn validate_vault(vault_path: &Path) -> Result<()> {
    if !vault_path.is_absolute() {
        bail!(
            "la ruta del vault debe ser absoluta: {}",
            vault_path.display()
        );
    }
    for path in std::iter::once(vault_path.to_path_buf()).chain(
        BASE_DIRECTORIES
            .iter()
            .map(|directory| vault_path.join(directory)),
    ) {
        let metadata = fs::metadata(&path)
            .with_context(|| format!("no se pudo inspeccionar {}", path.display()))?;
        if !metadata.is_dir() {
            bail!("{} existe, pero no es un directorio", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_missing_and_file_roots() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        fs::write(&file, "not a directory").unwrap();
        let missing = temp.path().join("missing");
        for (path, detail) in [
            (Path::new("relative"), "absoluta"),
            (missing.as_path(), "no se pudo inspeccionar"),
            (file.as_path(), "no es un directorio"),
        ] {
            let error = validate_vault(path).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains(detail), "{message}");
            assert!(message.contains(&path.display().to_string()));
            if path == missing {
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    std::io::ErrorKind::NotFound
                );
            }
        }
        assert!(!missing.exists());
    }

    #[test]
    fn requires_every_child_directory_without_repair() {
        for expected in BASE_DIRECTORIES {
            for replace_with_file in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let rejected = temp.path().join(expected);
                for directory in BASE_DIRECTORIES {
                    let path = temp.path().join(directory);
                    if !path.starts_with(&rejected) {
                        fs::create_dir_all(path).unwrap();
                    }
                }
                if replace_with_file {
                    fs::write(&rejected, "not a directory").unwrap();
                }
                let error = validate_vault(temp.path()).unwrap_err();
                let message = format!("{error:#}");
                assert!(
                    message.contains(&rejected.display().to_string()),
                    "{message}"
                );
                if replace_with_file {
                    assert!(message.contains("no es un directorio"));
                    assert!(rejected.is_file());
                } else {
                    assert_eq!(
                        error.downcast_ref::<std::io::Error>().unwrap().kind(),
                        std::io::ErrorKind::NotFound
                    );
                    assert!(!rejected.exists());
                }
            }
        }
    }
}
