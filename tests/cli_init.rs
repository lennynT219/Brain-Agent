use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::tempdir;

const BASE_DIRECTORIES: [&str; 8] = [
    "00-limbo",
    "01-proyectos",
    "02-areas",
    "03-recursos",
    "04-archivo",
    "99-sistema",
    "99-sistema/plantillas",
    "99-sistema/documentacion",
];

fn brain(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_brain"));
    command.current_dir(cwd).stdin(Stdio::null());
    command
}

fn init(cwd: &Path, path: &Path, dry_run: bool) -> Output {
    let mut command = brain(cwd);
    command.arg("init").arg(path);
    if dry_run {
        command.arg("--dry-run");
    }
    command.output().expect("run brain init")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn entries(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .expect("read directory")
        .map(|entry| {
            entry
                .expect("read entry")
                .file_name()
                .into_string()
                .expect("test entry is UTF-8")
        })
        .collect();
    names.sort();
    names
}

fn assert_structure(root: &Path) {
    for directory in BASE_DIRECTORIES {
        assert!(root.join(directory).is_dir(), "missing {directory}");
    }
}

#[test]
fn help_lists_init_without_writes() {
    let temp = tempdir().unwrap();
    let output = brain(temp.path()).arg("--help").output().unwrap();
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("init"));
    assert!(entries(temp.path()).is_empty());
}

#[test]
fn init_help_lists_path_and_dry_run_without_writes() {
    let temp = tempdir().unwrap();
    let output = brain(temp.path())
        .args(["init", "--help"])
        .output()
        .unwrap();
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PATH"));
    assert!(stdout.contains("--dry-run"));
    assert!(entries(temp.path()).is_empty());
}

#[test]
fn init_creates_all_eight_directories() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    assert_success(&init(temp.path(), &root, false));
    assert_structure(&root);
    assert_eq!(entries(&root).len(), 6);
    assert_eq!(entries(&root.join("99-sistema")).len(), 2);
    for directory in BASE_DIRECTORIES {
        if directory != "99-sistema" {
            assert!(entries(&root.join(directory)).is_empty());
        }
    }
}

#[test]
fn dry_run_does_not_create_root() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    assert_success(&init(temp.path(), &root, true));
    assert!(!root.exists());
    assert!(entries(temp.path()).is_empty());
}

#[test]
fn dry_run_preserves_existing_content_without_creating_structure() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::create_dir_all(root.join("personal")).unwrap();
    let note = root.join("personal/note.md");
    fs::write(&note, b"Existing note\n").unwrap();
    fs::write(root.join("README.md"), b"Existing root content\n").unwrap();

    assert_success(&init(temp.path(), &root, true));

    assert_eq!(fs::read(&note).unwrap(), b"Existing note\n");
    assert_eq!(
        fs::read(root.join("README.md")).unwrap(),
        b"Existing root content\n"
    );
    assert_eq!(entries(&root), ["README.md", "personal"]);
    assert_eq!(entries(&root.join("personal")), ["note.md"]);
    for directory in BASE_DIRECTORIES {
        assert!(!root.join(directory).exists(), "created {directory}");
    }
}

#[test]
fn repeated_init_preserves_note() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    assert_success(&init(temp.path(), &root, false));
    let note = root.join("00-limbo/note.md");
    fs::write(&note, b"Keep this note\n").unwrap();

    assert_success(&init(temp.path(), &root, false));

    assert_structure(&root);
    assert_eq!(fs::read(&note).unwrap(), b"Keep this note\n");
    assert_eq!(entries(&root.join("00-limbo")), ["note.md"]);
}

#[test]
fn init_completes_partial_structure_preserving_note() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::create_dir_all(root.join("00-limbo")).unwrap();
    let note = root.join("00-limbo/note.md");
    fs::write(&note, b"Partial vault note\n").unwrap();

    assert_success(&init(temp.path(), &root, false));

    assert_structure(&root);
    assert_eq!(fs::read(&note).unwrap(), b"Partial vault note\n");
}

#[test]
fn relative_path_is_resolved_against_subprocess_working_directory() {
    let temp = tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    fs::create_dir(&cwd).unwrap();

    assert_success(&init(&cwd, Path::new("vault"), false));

    assert_structure(&cwd.join("vault"));
    assert_eq!(entries(temp.path()), ["workspace"]);
    assert_eq!(entries(&cwd), ["vault"]);
}

#[test]
fn root_file_conflict_fails_without_modifying_content_or_creating_directories() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::write(&root, b"Root is a file\n").unwrap();

    let output = init(temp.path(), &root, false);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(&root).unwrap(), b"Root is a file\n");
    assert_eq!(entries(temp.path()), ["vault"]);
    for directory in BASE_DIRECTORIES {
        assert!(!root.join(directory).exists(), "created {directory}");
    }
}

#[test]
fn nested_file_conflict_fails_before_creating_any_missing_base_directories() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::create_dir_all(root.join("99-sistema")).unwrap();
    let conflict = root.join("99-sistema/documentacion");
    fs::write(&conflict, b"Nested target is a file\n").unwrap();

    let output = init(temp.path(), &root, false);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(&conflict).unwrap(), b"Nested target is a file\n");
    assert_eq!(entries(&root), ["99-sistema"]);
    assert_eq!(entries(&root.join("99-sistema")), ["documentacion"]);
    for directory in BASE_DIRECTORIES {
        if directory != "99-sistema" && directory != "99-sistema/documentacion" {
            assert!(!root.join(directory).exists(), "created {directory}");
        }
    }
}
