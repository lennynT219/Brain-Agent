use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub config_version: u32,
    pub vault_path: PathBuf,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.config_version != CONFIG_VERSION {
            bail!(
                "versión de configuración no compatible: {}; se requiere {}",
                self.config_version,
                CONFIG_VERSION
            );
        }
        crate::vault::validate_vault(&self.vault_path)
    }
}

pub fn load_config() -> Result<Config> {
    load_config_from(&config_file_path()?)
}

fn load_config_from(path: &Path) -> Result<Config> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("no se pudo leer la configuración en {}", path.display()));
    let input = match input {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|source| source.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Err(
                error.context("falta seleccionar un vault; usa brain config set-vault <ruta>")
            );
        }
        result => result?,
    };
    let config: Config =
        toml::from_str(&input).with_context(|| format!("TOML inválido en {}", path.display()))?;
    config
        .validate()
        .with_context(|| format!("configuración inválida en {}", path.display()))?;
    Ok(config)
}

pub fn save_config(config: &Config, replace_existing: bool) -> Result<PathBuf> {
    let path = config_file_path()?;
    save_config_to(config, &path, replace_existing)?;
    Ok(path)
}

fn save_config_to(config: &Config, path: &Path, replace_existing: bool) -> Result<()> {
    config.validate().context("configuración nueva inválida")?;
    let serialized =
        toml::to_string_pretty(config).context("no se pudo serializar la configuración")?;
    if !path.is_absolute() || path.file_name().is_none() {
        bail!("destino de configuración no válido: {}", path.display());
    }
    let parent = path
        .parent()
        .context("el destino requiere un directorio padre")?;
    let check_target = || -> Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                bail!("el destino no es un archivo regular: {}", path.display());
            }
            Ok(_) if !replace_existing => {
                bail!(
                    "la configuración ya existe; se requiere autorización: {}",
                    path.display()
                );
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("no se pudo inspeccionar {}", path.display()))
            }
        }
    };
    check_target()?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .with_context(|| format!("no se pudo crear el directorio {}", parent.display()))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "no se pudo preparar la configuración en {}",
            parent.display()
        )
    })?;
    staged
        .write_all(serialized.as_bytes())
        .context("no se pudo escribir la configuración temporal")?;
    staged
        .flush()
        .context("no se pudo vaciar la configuración temporal")?;
    staged
        .as_file()
        .sync_all()
        .context("no se pudo sincronizar la configuración temporal")?;
    // Recheck the leaf, not a concurrency guarantee; the parent must be trusted.
    check_target()?;
    let published = if replace_existing {
        staged.persist(path)
    } else {
        staged.persist_noclobber(path)
    };
    published.map(|_| ()).map_err(|failure| {
        let tempfile::PersistError { error, file } = failure;
        drop(file);
        anyhow::Error::new(error).context(format!(
            "no se pudo publicar la configuración en {}",
            path.display()
        ))
    })
}

pub fn config_file_path() -> anyhow::Result<PathBuf> {
    resolve_config_file_path(
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
        cfg!(windows),
    )
}

fn resolve_config_file_path(
    home: Option<PathBuf>,
    userprofile: Option<PathBuf>,
    windows: bool,
) -> anyhow::Result<PathBuf> {
    let (base, variable) = if windows {
        (userprofile, "USERPROFILE")
    } else {
        (home, "HOME")
    };
    let base = base.filter(|path| path.is_absolute()).ok_or_else(|| {
        anyhow::anyhow!(
            "No se puede resolver la ruta de configuración: se requiere {variable} con una ruta absoluta válida"
        )
    })?;

    Ok(base.join(".config").join("brain").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_config(root: &Path) -> Config {
        for directory in crate::vault::BASE_DIRECTORIES {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        Config {
            config_version: CONFIG_VERSION,
            vault_path: root.to_path_buf(),
        }
    }

    #[test]
    fn saves_roundtrip_without_clobbering_and_replaces_when_authorized() {
        let temp = tempfile::tempdir().unwrap();
        let config = complete_config(&temp.path().join("vault"));
        let path = temp.path().join("new/brain/config.toml");
        save_config_to(&config, &path, false).unwrap();
        assert_eq!(
            load_config_from(&path).unwrap().vault_path,
            config.vault_path
        );
        let original = b"broken old config\n";
        fs::write(&path, original).unwrap();
        assert!(save_config_to(&config, &path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        save_config_to(&config, &path, true).unwrap();
        assert_eq!(
            load_config_from(&path).unwrap().vault_path,
            config.vault_path
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_new_config_never_mutates_destination_or_creates_parent() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = complete_config(&temp.path().join("vault"));
        config.config_version += 1;
        let existing = temp.path().join("config.toml");
        let missing = temp.path().join("missing/config.toml");
        fs::write(&existing, b"original").unwrap();
        for replace in [false, true] {
            for path in [&existing, &missing] {
                assert!(save_config_to(&config, path, replace).is_err());
                assert_eq!(fs::read(&existing).unwrap(), b"original");
                assert!(!missing.parent().unwrap().exists());
            }
        }
    }

    #[test]
    fn rejects_nonregular_and_unusable_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let config = complete_config(&temp.path().join("vault"));
        let directory = temp.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let file = temp.path().join("parent-file");
        fs::write(&file, b"untouched").unwrap();
        for replace in [false, true] {
            for path in [
                &directory,
                &file.join("config.toml"),
                Path::new("relative.toml"),
                Path::new(""),
            ] {
                assert!(save_config_to(&config, path, replace).is_err());
            }
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
        assert_eq!(fs::read(file).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn serialization_failure_precedes_all_destination_mutations() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let temp = tempfile::tempdir().unwrap();
        let config = complete_config(&temp.path().join(OsString::from_vec(vec![0xff])));
        config.validate().unwrap();
        let existing = temp.path().join("config.toml");
        let missing = temp.path().join("missing/config.toml");
        fs::write(&existing, b"original").unwrap();
        for replace in [false, true] {
            for path in [&existing, &missing] {
                let error = save_config_to(&config, path, replace).unwrap_err();
                assert!(error.downcast_ref::<toml::ser::Error>().is_some());
                assert_eq!(fs::read(&existing).unwrap(), b"original");
                assert!(!missing.parent().unwrap().exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_leaf_symlinks_including_dangling_ones() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let config = complete_config(&temp.path().join("vault"));
        let target = temp.path().join("target");
        fs::write(&target, b"untouched").unwrap();
        for (name, destination) in [
            ("link", target.clone()),
            ("dangling", temp.path().join("absent")),
        ] {
            let link = temp.path().join(name);
            symlink(&destination, &link).unwrap();
            for replace in [false, true] {
                assert!(save_config_to(&config, &link, replace).is_err());
                assert_eq!(fs::read_link(&link).unwrap(), destination);
            }
        }
        assert_eq!(fs::read(target).unwrap(), b"untouched");
        assert!(!temp.path().join("absent").exists());
    }

    #[cfg(unix)]
    #[test]
    fn new_directories_are_private_and_existing_permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let config = complete_config(&temp.path().join("vault"));
        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o750)).unwrap();
        let original = fs::metadata(&existing).unwrap().permissions().mode();
        save_config_to(&config, &existing.join("config.toml"), false).unwrap();
        assert_eq!(
            fs::metadata(&existing).unwrap().permissions().mode(),
            original
        );
        save_config_to(&config, &temp.path().join("new/nested/config.toml"), false).unwrap();
        for directory in ["new", "new/nested"] {
            let mode = fs::metadata(temp.path().join(directory))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
            assert_eq!(mode & 0o700, 0o700);
        }
    }

    #[test]
    fn rejects_unsupported_version_before_vault_validation() {
        let config = Config {
            config_version: CONFIG_VERSION + 1,
            vault_path: PathBuf::from("relative"),
        };
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("versión"));
    }

    #[test]
    fn loads_complete_vault_without_normalizing_stored_path() {
        let temp = tempfile::tempdir().unwrap();
        for directory in crate::vault::BASE_DIRECTORIES {
            fs::create_dir_all(temp.path().join(directory)).unwrap();
        }
        let config = Config {
            config_version: CONFIG_VERSION,
            vault_path: temp.path().join("99-sistema/.."),
        };
        let path = temp.path().join("config.toml");
        let input = toml::to_string(&config).unwrap();
        fs::write(&path, &input).unwrap();
        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.config_version, CONFIG_VERSION);
        assert_eq!(loaded.vault_path, config.vault_path);
        assert_eq!(fs::read_to_string(path).unwrap(), input);
    }

    #[test]
    fn reports_missing_config_and_other_io_errors_with_sources() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.toml");
        for path in [&missing, temp.path()] {
            let error = load_config_from(path).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains(&path.display().to_string()));
            let source = error.downcast_ref::<std::io::Error>().unwrap();
            assert_eq!(
                message.contains("brain config set-vault <ruta>"),
                source.kind() == std::io::ErrorKind::NotFound
            );
        }
        assert!(!missing.exists());
    }

    #[test]
    fn distinguishes_toml_and_semantic_errors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        for (input, category, detail) in [
            ("config_version = [", "TOML inválido", ""),
            (
                "config_version = '1'\nvault_path = 'relative'",
                "TOML inválido",
                "",
            ),
            (
                "config_version = 2\nvault_path = 'relative'",
                "configuración inválida",
                "versión",
            ),
            (
                "config_version = 1\nvault_path = 'relative'",
                "configuración inválida",
                "absoluta",
            ),
        ] {
            fs::write(&path, input).unwrap();
            let error = load_config_from(&path).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains(category) && message.contains(detail),
                "{message}"
            );
            assert!(message.contains(&path.display().to_string()));
            assert!(error.chain().count() > 1);
            if category == "TOML inválido" {
                assert!(error.downcast_ref::<toml::de::Error>().is_some());
            }
            assert_eq!(fs::read_to_string(&path).unwrap(), input);
        }
    }

    #[test]
    fn selects_platform_home_lexically_without_creating_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("José Pérez");
        let profile = temp.path().join("perfil Windows");
        for windows in [false, true] {
            let path = resolve_config_file_path(Some(home.clone()), Some(profile.clone()), windows)
                .unwrap();
            let selected = if windows { &profile } else { &home };
            assert_eq!(path, selected.join(".config/brain/config.toml"));
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn rejects_unusable_selected_home_without_other_platform_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let valid = Some(temp.path().to_path_buf());
        for windows in [false, true] {
            for invalid in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
                let (home, profile) = if windows {
                    (valid.clone(), invalid)
                } else {
                    (invalid, valid.clone())
                };
                let error = resolve_config_file_path(home, profile, windows).unwrap_err();
                let variable = if windows { "USERPROFILE" } else { "HOME" };
                assert_eq!(
                    error.to_string(),
                    format!(
                        "No se puede resolver la ruta de configuración: se requiere {variable} con una ruta absoluta válida"
                    )
                );
                assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
            }
        }
    }

    #[test]
    fn parses_valid_config() {
        let input = "config_version = 1\nvault_path = '/vault'\n";

        let config: Config = toml::from_str(input).unwrap();

        assert_eq!(config.config_version, CONFIG_VERSION);
        assert_eq!(config.vault_path, PathBuf::from("/vault"));
    }

    #[test]
    fn roundtrips_config() {
        let config = Config {
            config_version: CONFIG_VERSION,
            vault_path: PathBuf::from("/vault with spaces"),
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();

        assert_eq!(parsed.config_version, config.config_version);
        assert_eq!(parsed.vault_path, config.vault_path);
    }

    #[test]
    fn rejects_missing_required_fields() {
        for input in ["", "config_version = 1\n", "vault_path = '/vault'\n"] {
            assert!(toml::from_str::<Config>(input).is_err(), "{input}");
        }
    }

    #[test]
    fn rejects_wrong_field_types() {
        for input in [
            "config_version = '1'\nvault_path = '/vault'\n",
            "config_version = 1\nvault_path = 42\n",
        ] {
            assert!(toml::from_str::<Config>(input).is_err(), "{input}");
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let input = "config_version = 1\nvault_path = '/vault'\nextra = true\n";

        assert!(toml::from_str::<Config>(input).is_err());
    }
}
