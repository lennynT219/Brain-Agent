//! Prepared regression coverage only. All fixtures use their own vault/profile.
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf, process::{Command, Stdio}};
use tempfile::TempDir;
use uuid::Uuid;

const BODY: &str = "## Summary\nResumen del proyecto.\n## Objective\nObjetivo.\n## Scope\nAlcance acotado.\n";
struct Fixture { _temp: TempDir, cwd: PathBuf, vault: PathBuf, home: PathBuf }
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let f = Self { cwd: temp.path().join("context"), vault: temp.path().join("vault"), home: temp.path().join("home"), _temp: temp };
        fs::create_dir(&f.cwd).unwrap();
        fs::create_dir(&f.home).unwrap();
        assert!(f.command().arg("init").arg(&f.vault).output().unwrap().status.success());
        assert!(f.command().args(["config", "set-vault"]).arg(&f.vault).output().unwrap().status.success());
        f
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_brain"));
        cmd.current_dir(&self.cwd).env("HOME", &self.home).env("USERPROFILE", &self.home);
        cmd
    }
    fn call(&self, endpoint: &str, op: &str, arguments: Value) -> Value {
        let id = Uuid::new_v4();
        let mut child = self.command().arg(endpoint).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let request = json!({"protocol_version":1,"request_id":id,"operation":op,"arguments":arguments});
        child.stdin.take().unwrap().write_all(&serde_json::to_vec(&request).unwrap()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["request_id"], id.to_string());
        if endpoint == "projects" { assert_eq!(response["operation"], op); }
        response["result"].clone()
    }
    fn status(&self) -> Value { self.call("projects", "status", json!({})) }
    fn preview(&self) -> Value { self.call("projects", "preview_create", json!({"title":"Proyecto español seguro", "body":BODY})) }
    fn create(&self, preview: &Value) -> Value {
        self.call("projects", "create_and_bind", json!({"preview":preview["preview"],"expected":preview["expected"],"human_approved":true}))
    }
    fn memory_create(&self) -> Value {
        self.call("memory", "create", json!({"context":{"working_directory":self.cwd,"authorized_boundary":self.cwd},
            "title":"Contexto útil", "sections":["Objetivo","Decisiones","Estado","Evidencia","Próximos pasos"], "summary":"Se registró un resultado útil."}))
    }
    fn existing(&self, folder: &str, id: Uuid) -> PathBuf {
        let dir = self.vault.join("01-proyectos").join(folder);
        fs::create_dir_all(dir.join("notas")).unwrap();
        let path = dir.join("Proyecto.md");
        fs::write(&path, format!("---\nbrain_type: project\nbrain_version: 1\nbrain_id: {id}\ntitle: Proyecto existente\n---\nContenido humano sin reescribir.\n")).unwrap();
        path
    }
}

fn snapshot_tree(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &std::path::Path, path: &std::path::Path,
        entries: &mut std::collections::BTreeMap<PathBuf, Option<Vec<u8>>>) {
        if path.is_dir() {
            entries.insert(path.strip_prefix(root).unwrap().to_path_buf(), None);
            for entry in fs::read_dir(path).unwrap() { visit(root, &entry.unwrap().path(), entries); }
        } else {
            entries.insert(path.strip_prefix(root).unwrap().to_path_buf(), Some(fs::read(path).unwrap()));
        }
    }
    let mut entries = std::collections::BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[test]
fn bind_rejects_changed_confirmed_project_without_any_filesystem_effects() {
    for with_memory in [false, true] {
        for changed in ["title", "path", "identity"] {
            let f = Fixture::new();
            if with_memory { assert_eq!(f.memory_create()["status"], "saved"); }
            let id = Uuid::new_v4();
            let path = f.existing("Selected", id);
            // Missing notas/ must not be recreated by a rejected confirmation.
            fs::remove_dir(path.parent().unwrap().join("notas")).unwrap();
            let shown = f.call("projects", "read", json!({"project_id":id}))["project"].clone();
            let baseline = f.status()["expected"].clone();
            match changed {
                "title" => {
                    let raw = fs::read_to_string(&path).unwrap();
                    fs::write(&path, raw.replace("title: Proyecto existente", "title: Título posterior")).unwrap();
                }
                "path" => fs::rename(path.parent().unwrap(), f.vault.join("01-proyectos/Moved")).unwrap(),
                "identity" => {
                    let raw = fs::read_to_string(&path).unwrap();
                    fs::write(&path, raw.replace(&id.to_string(), &Uuid::new_v4().to_string())).unwrap();
                }
                _ => unreachable!(),
            }
            let cwd_before = snapshot_tree(&f.cwd);
            let vault_before = snapshot_tree(&f.vault);
            let result = f.call("projects", "bind", json!({"project_id":id,"expected_project":shown,
                "expected":baseline,"human_approved":true}));
            assert_eq!(result["status"], "conflict", "{changed}, memory={with_memory}: {result}");
            assert_eq!(snapshot_tree(&f.cwd), cwd_before);
            assert_eq!(snapshot_tree(&f.vault), vault_before);
        }
    }
}

#[test]
fn bind_requires_expected_project_and_matching_request_identity_without_fallback() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    f.existing("Selected", id);
    let shown = f.call("projects", "read", json!({"project_id":id}))["project"].clone();
    let baseline = f.status()["expected"].clone();
    let cwd_before = snapshot_tree(&f.cwd);
    let vault_before = snapshot_tree(&f.vault);
    assert_eq!(f.call("projects", "bind", json!({"project_id":id,"expected":baseline,
        "human_approved":true}))["status"], "invalid");
    assert_eq!(f.call("projects", "bind", json!({"project_id":Uuid::new_v4(),"expected_project":shown,
        "expected":baseline,"human_approved":true}))["status"], "conflict");
    assert_eq!(snapshot_tree(&f.cwd), cwd_before);
    assert_eq!(snapshot_tree(&f.vault), vault_before);
}

#[test]
fn preview_has_no_writes_and_confirmed_create_defers_memory_then_inherits_project() {
    let f = Fixture::new();
    let before = f.status();
    assert_eq!(before["status"], "unbound");
    assert_eq!(before["binding"], "none");
    assert_eq!(before["allow_write"], false);
    let p = f.preview();
    assert_eq!(p["status"], "preview");
    assert!(!f.cwd.join(".brain.toml").exists());
    assert_eq!(fs::read_dir(f.vault.join("01-proyectos")).unwrap().count(), 0);
    let receipt = f.create(&p);
    assert_eq!(receipt["status"], "bound");
    assert_eq!(receipt["memory"]["status"], "deferred");
    assert_eq!(receipt["created"], true);
    assert!(!f.vault.join("05-memorias").exists());
    let status = f.status();
    assert_eq!(status["binding"], "unified");
    assert_eq!(status["allow_write"], true);
    let file = f.vault.join(status["principal"]["path"].as_str().unwrap());
    assert!(file.parent().unwrap().join("notas").is_dir());
    let saved = f.memory_create();
    assert_eq!(saved["status"], "saved");
    let read = f.call("memory", "read", json!({"memory_id":saved["memory_id"]}));
    assert!(read["accepted_markdown"].as_str().unwrap().contains(p["preview"]["project_id"].as_str().unwrap()));
    let resolved = f.call("memory", "resolve", json!({"working_directory":f.cwd,"authorized_boundary":f.cwd}));
    assert_eq!(resolved["binding"]["binding_version"], 1);
    assert_eq!(resolved["binding"]["memory_id"], saved["memory_id"]);
    assert_eq!(resolved["binding"]["memory_subfolders"], json!([]));
    assert_ne!(f.create(&p)["status"], "bound");
    assert_eq!(f.call("projects", "list", json!({}))["projects"].as_array().unwrap().len(), 1);
}
#[test]
fn stale_or_unapproved_creation_does_not_allocate_project() {
    let f = Fixture::new();
    let p = f.preview();
    assert_eq!(f.call("projects", "create_and_bind", json!({"preview":p["preview"],"expected":p["expected"],"human_approved":false}))["status"], "denied");
    let saved = f.memory_create();
    assert_eq!(saved["status"], "saved");
    assert_eq!(f.create(&p)["status"], "conflict");
    assert_eq!(fs::read_dir(f.vault.join("01-proyectos")).unwrap().count(), 0);
}
#[test]
fn existing_project_binding_preserves_document_memory_identity_and_history() {
    let f = Fixture::new();
    let saved = f.memory_create();
    let id = Uuid::new_v4();
    let path = f.existing("Existente", id);
    let raw = fs::read(&path).unwrap();
    let memory_dir = f.vault.join("05-memorias").join(saved["memory_id"].as_str().unwrap());
    let history = fs::read_to_string(memory_dir.join("Historial.md")).unwrap();
    let before = f.status();
    let selected = f.call("projects", "read", json!({"project_id":id}));
    let bound = f.call("projects", "bind", json!({"project_id":id,"expected_project":selected["project"],"expected":before["expected"],"human_approved":true}));
    assert_eq!(bound["status"], "bound");
    assert_eq!(bound["created"], false);
    assert_eq!(bound["memory"]["memory_id"], saved["memory_id"]);
    assert_eq!(bound["memory"]["revision"], 2);
    assert_eq!(fs::read(path).unwrap(), raw);
    assert!(fs::read_to_string(memory_dir.join("Historial.md")).unwrap().starts_with(&history));
    let draft = f.call("projects", "draft_context", json!({}));
    assert_eq!(draft["history"]["status"], "found");
    assert_eq!(draft["history"]["format"], 2);
    assert!(draft["history"]["returned_bytes"].as_u64().unwrap() <= 32768);
}
#[test]
fn manual_and_partial_memory_block_all_project_publication() {
    for partial in [false, true] {
        let f = Fixture::new();
        let saved = f.memory_create();
        let p = f.preview();
        let dir = f.vault.join("05-memorias").join(saved["memory_id"].as_str().unwrap());
        if partial { fs::remove_file(dir.join("Historial.md")).unwrap(); }
        else {
            let path = dir.join("Memoria.md");
            let raw = fs::read_to_string(&path).unwrap();
            fs::write(path, raw.replace("Objetivo", "Objetivo editado externamente")).unwrap();
        }
        let marker = fs::read(f.cwd.join(".brain.toml")).unwrap();
        let result = f.create(&p);
        assert_ne!(result["status"], "bound");
        assert_eq!(fs::read(f.cwd.join(".brain.toml")).unwrap(), marker);
        assert_eq!(fs::read_dir(f.vault.join("01-proyectos")).unwrap().count(), 0);
    }
}
#[test]
fn additional_project_notes_are_explicit_read_only_and_v2_does_not_fallback() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let path = f.existing("Additional", id);
    let note_id = Uuid::new_v4();
    fs::write(path.parent().unwrap().join("notas/Nota.md"), format!("---\nbrain_type: note\nbrain_version: 1\nbrain_id: {note_id}\ntitle: Nota\n---\nContenido adicional\n")).unwrap();
    assert_eq!(f.call("notes", "list", json!({"project_id":id}))["status"], "listed");
    assert_eq!(f.call("notes", "read", json!({"project_id":id,"note_id":note_id}))["status"], "found");
    assert_eq!(f.call("notes", "create", json!({"project_id":id,"title":"No autorizada","body":"x"}))["status"], "unbound");
    fs::write(f.cwd.join(".brain-notes.toml"), format!("binding_version=1\nproject_id='{id}'\nproject_path='01-proyectos/Additional/Proyecto.md'\nallow_write=true\n")).unwrap();
    assert_eq!(f.status()["binding"], "legacy");
    fs::write(f.cwd.join(".brain.toml"), "binding_version=2\n").unwrap();
    assert_ne!(f.call("notes", "list", json!({}))["status"], "listed");
    assert_ne!(f.call("notes", "create", json!({"project_id":id,"title":"No autorizada","body":"x"}))["status"], "created");
}
#[test]
fn inactive_projects_are_not_listed_and_duplicate_identity_fails_closed() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let active = f.existing("Active", id);
    let old = f.existing("Archived", Uuid::new_v4());
    let raw = fs::read_to_string(&old).unwrap().replace("title:", "project_status: archived\ntitle:");
    fs::write(old, raw).unwrap();
    assert_eq!(f.call("projects", "list", json!({}))["projects"].as_array().unwrap().len(), 1);
    f.existing("Duplicate", id);
    assert_eq!(f.call("projects", "list", json!({}))["status"], "conflict");
    assert!(active.is_file());
}

#[test]
fn changed_vault_or_full_area_blocks_creation_and_keeps_preview_read_only() {
    let f = Fixture::new();
    let p = f.preview();
    let next_vault = f._temp.path().join("other-vault");
    assert!(f.command().arg("init").arg(&next_vault).output().unwrap().status.success());
    assert!(f.command().args(["config", "set-vault"]).arg(&next_vault).arg("--yes").output().unwrap().status.success());
    assert_eq!(f.create(&p)["status"], "conflict");
    assert!(!f.cwd.join(".brain.toml").exists());
    assert_eq!(fs::read_dir(next_vault.join("01-proyectos")).unwrap().count(), 0);

    let full = Fixture::new();
    for n in 0..256 { fs::write(full.vault.join("01-proyectos").join(format!("entry-{n}.txt")), "reserved").unwrap(); }
    assert_eq!(full.preview()["status"], "too_large");
    assert!(!full.cwd.join(".brain.toml").exists());
    assert_eq!(fs::read_dir(full.vault.join("01-proyectos")).unwrap().count(), 256);
}

#[test]
fn v2_ignores_inactive_legacy_and_first_memory_create_rejects_moved_project() {
    let f = Fixture::new();
    assert_eq!(f.create(&f.preview())["status"], "bound");
    // Inactive legacy cannot override or break a valid consolidated principal.
    fs::create_dir(f.cwd.join(".brain-notes.toml")).unwrap();
    let status = f.status();
    assert_eq!(status["binding"], "unified");
    assert!(status["expected"]["legacy_marker"].is_null());
    assert_eq!(f.call("notes", "list", json!({}))["status"], "listed");
    let project_path = f.vault.join(status["principal"]["path"].as_str().unwrap());
    fs::rename(project_path.parent().unwrap(), f.vault.join("01-proyectos/Moved")).unwrap();
    assert_eq!(f.memory_create()["status"], "conflict");
    assert!(!f.vault.join("05-memorias").exists());
    assert!(!f.cwd.join(".brain.pending.toml").exists());
}

#[test]
fn new_project_files_use_literal_title_and_confirmation_rejects_a_different_filename() {
    for title in ["StelaShell", "Análisis de Datos", "Árbol  Español"] {
        let f = Fixture::new();
        let preview = f.call("projects", "preview_create", json!({"title":title,"body":BODY}));
        assert_eq!(preview["status"], "preview");
        let path = preview["preview"]["path"].as_str().unwrap();
        assert_eq!(std::path::Path::new(path).file_name().unwrap().to_str().unwrap(), format!("{title}.md"));
        let parent = std::path::Path::new(path).parent().unwrap();
        let mut tampered = preview.clone();
        tampered["preview"]["path"] = Value::String(format!("{}/Proyecto.md", parent.to_str().unwrap()));
        let cwd_before = snapshot_tree(&f.cwd);
        let vault_before = snapshot_tree(&f.vault);
        assert_eq!(f.create(&tampered)["status"], "invalid");
        assert_eq!(snapshot_tree(&f.cwd), cwd_before);
        assert_eq!(snapshot_tree(&f.vault), vault_before);
        let receipt = f.create(&preview);
        assert_eq!(receipt["status"], "bound");
        assert!(f.vault.join(path).is_file());
        assert_eq!(receipt["project"]["title"], title);
        assert_eq!(receipt["project"]["path"], path);
    }
}

#[test]
fn nonportable_new_titles_are_rejected_without_writes_and_existing_names_remain_valid() {
    let f = Fixture::new();
    for title in ["Nombre/Parte".to_owned(), "Nombre\\Parte".to_owned(), "CON".to_owned(),
        "NUL.txt".to_owned(), "LPT¹".to_owned(), "CONIN$".to_owned(), "CON .txt".to_owned(),
        "Nombre ".to_owned(), "a".repeat(253), format!("{}a", "á".repeat(126))] {
        let cwd_before = snapshot_tree(&f.cwd);
        let vault_before = snapshot_tree(&f.vault);
        let result = f.call("projects", "preview_create", json!({"title":title,"body":BODY}));
        assert_eq!(result["status"], "invalid");
        assert_eq!(result["reason"], "INVALID_PROJECT_FILENAME");
        assert_eq!(snapshot_tree(&f.cwd), cwd_before);
        assert_eq!(snapshot_tree(&f.vault), vault_before);
    }
    let id = Uuid::new_v4();
    let path = f.existing("Existing", id);
    let raw = fs::read_to_string(&path).unwrap().replace("title: Proyecto existente", "title: Título / heredado");
    fs::write(&path, &raw).unwrap();
    fs::write(f.cwd.join(".brain-notes.toml"), format!("binding_version=1\nproject_id='{id}'\nproject_path='01-proyectos/Existing/Proyecto.md'\nallow_write=false\n")).unwrap();
    assert_eq!(f.status()["principal"]["title"], "Título / heredado");
    assert_eq!(f.call("projects", "read", json!({"project_id":id}))["markdown"], raw);
    assert_eq!(f.call("notes", "list", json!({}))["status"], "listed");
    assert_eq!(fs::read_to_string(path).unwrap(), raw);
}

#[test]
fn draft_context_bounds_selected_legacy_history_without_cutting_utf8() {
    let f = Fixture::new();
    let saved = f.call("memory", "create", json!({"context":{"working_directory":f.cwd,"authorized_boundary":f.cwd},
        "title":"Contexto útil", "sections":["Objetivo","Decisiones","Estado","áéíóú".repeat(10000),"Próximos pasos"]}));
    assert_eq!(saved["status"], "saved");
    let dir = f.vault.join("05-memorias").join(saved["memory_id"].as_str().unwrap());
    let raw = fs::read_to_string(dir.join("Historial.md")).unwrap();
    let before = fs::read(dir.join("Memoria.md")).unwrap();
    let draft = f.call("projects", "draft_context", json!({}));
    assert_eq!(draft["history"]["format"], 1);
    assert_eq!(draft["history"]["truncated"], true);
    let returned = draft["history"]["markdown"].as_str().unwrap();
    let mut bytes = 0;
    let expected: String = raw.chars().rev().take_while(|c| {
        if bytes + c.len_utf8() > 32768 { false } else { bytes += c.len_utf8(); true }
    }).collect::<Vec<_>>().into_iter().rev().collect();
    assert_eq!(returned, expected);
    assert_eq!(draft["history"]["returned_bytes"], bytes);
    assert_eq!(draft["history"]["total_bytes"], raw.len());
    assert_eq!(fs::read(dir.join("Memoria.md")).unwrap(), before);
    assert_ne!(f.call("projects", "draft_context", json!({"memory_id":Uuid::new_v4()}))["status"], "found");
}
