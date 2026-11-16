use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::tempdir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("misleading-xdg"))
        .stdin(Stdio::null())
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn config_path(root: &Path) -> std::path::PathBuf {
    root.join("home/.config/brain/config.toml")
}

fn failure(output: Output, expected: &str) {
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
}

#[test]
fn selection_ignores_xdg_and_uses_fixed_home() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    success(run(root, &["init", "vault"]));
    let output = run(root, &["config", "set-vault", "vault"]);
    let vault = root.join("vault").canonicalize().unwrap();
    let file = config_path(root);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "Vault seleccionado: {}\nArchivo de configuración: {}\n",
            vault.display(),
            file.display()
        )
    );
    assert!(file.is_file());
    assert!(!root.join("misleading-xdg").exists());
}

#[test]
fn relative_selection_and_show() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    success(run(root, &["init", "vault"]));
    let selected = success(run(root, &["config", "set-vault", "vault"]));
    let absolute = root.join("vault").canonicalize().unwrap();
    let file = config_path(root);
    let value: toml::Value = toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(value["config_version"].as_integer(), Some(1));
    assert_eq!(value["vault_path"].as_str(), absolute.to_str());
    let shown = success(run(root, &["config", "show"]));
    for output in [selected, shown] {
        assert!(output.contains(absolute.to_str().unwrap()));
        assert!(output.contains(file.to_str().unwrap()));
    }
}

#[test]
fn replacement_authorization_noop_and_repair() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    for vault in ["one", "two"] {
        success(run(root, &["init", vault]));
    }
    success(run(root, &["config", "set-vault", "one", "--yes"]));
    let file = config_path(root);
    let original = format!(
        "# preserve this comment\n{}",
        fs::read_to_string(&file).unwrap()
    );
    fs::write(&file, &original).unwrap();
    assert!(success(run(root, &["config", "set-vault", "./one"])).contains("sin cambios"));
    assert_eq!(fs::read(&file).unwrap(), original.as_bytes());
    failure(run(root, &["config", "set-vault", "two"]), "--yes");
    assert_eq!(fs::read(&file).unwrap(), original.as_bytes());
    success(run(root, &["config", "set-vault", "two", "--yes"]));
    assert!(success(run(root, &["config", "show"])).contains(root.join("two").to_str().unwrap()));
    fs::write(&file, "broken config").unwrap();
    success(run(root, &["config", "set-vault", "one", "--yes"]));
    assert!(success(run(root, &["config", "show"])).contains(root.join("one").to_str().unwrap()));
}

#[test]
fn queries_and_invalid_selections_never_write() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    for args in [
        vec!["config", "--help"],
        vec!["config", "set-vault", "--help"],
    ] {
        success(run(root, &args));
    }
    failure(run(root, &["config", "show"]), "falta seleccionar");
    failure(run(root, &["config", "show", "--yes"]), "--yes");
    assert!(!root.join("home").exists());
    assert!(!root.join("misleading-xdg").exists());
    fs::create_dir(root.join("incomplete")).unwrap();
    fs::write(root.join("not-directory"), "x").unwrap();
    let file = config_path(root);
    for prior in [None, Some("broken config")] {
        if let Some(bytes) = prior {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, bytes).unwrap();
            failure(run(root, &["config", "show"]), "TOML inválido");
        }
        for vault in ["missing", "incomplete", "not-directory"] {
            failure(run(root, &["config", "set-vault", vault, "--yes"]), "vault");
        }
        assert_eq!(fs::read_to_string(&file).ok().as_deref(), prior);
    }
}

#[test]
fn nonregular_destination_is_refused() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    success(run(root, &["init", "vault"]));
    let file = config_path(root);
    fs::create_dir_all(&file).unwrap();
    failure(
        run(root, &["config", "set-vault", "vault", "--yes"]),
        "archivo regular",
    );
    assert!(file.is_dir());
    #[cfg(unix)]
    {
        let other = tempdir().unwrap();
        success(run(other.path(), &["init", "vault"]));
        let leaf = config_path(other.path());
        fs::create_dir_all(leaf.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&file, &leaf).unwrap();
        failure(
            run(other.path(), &["config", "set-vault", "vault", "--yes"]),
            "archivo regular",
        );
        assert_eq!(fs::read_link(leaf).unwrap(), file);
    }
}
