use serde_json::{json, Value};
use std::{fs, io::Write, path::PathBuf, process::{Command, Stdio}};
use tempfile::TempDir;
use uuid::Uuid;

const BODY: &str = "## Summary\r\n\r\nResumen.\r\n\r\n## Objective\r\n\r\nObjetivo.\r\n\r\n## Scope\r\n\r\nAlcance.\r\n";
struct Fixture { _root: TempDir, home: PathBuf, cwd: PathBuf, vault: PathBuf, id: Uuid, path: PathBuf, header: String }
impl Fixture {
    fn new(unified: bool, allow_write: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let (home, cwd, vault) = (root.path().join("home"), root.path().join("work"), root.path().join("vault"));
        for dir in [&home, &cwd] { fs::create_dir(dir).unwrap(); }
        let id = Uuid::new_v4();
        let path = vault.join("01-proyectos/Prueba/Proyecto.md");
        let header = format!("---\r\nbrain_type: project\r\nbrain_version: 1\r\nbrain_id: {id}\r\ntitle: Proyecto legado\r\n# Propiedad humana\r\ntags: [trabajo]\r\n---\r\n");
        let f = Self { _root: root, home, cwd, vault, id, path, header };
        assert!(f.command().arg("init").arg(&f.vault).output().unwrap().status.success());
        assert!(f.command().args(["config", "set-vault"]).arg(&f.vault).output().unwrap().status.success());
        fs::create_dir(f.path.parent().unwrap()).unwrap();
        fs::write(&f.path, &f.header).unwrap();
        let binding = format!("project_id = \"{id}\"\nproject_path = \"01-proyectos/Prueba/Proyecto.md\"\nallow_write = {allow_write}\n");
        if unified {
            fs::write(f.cwd.join(".brain.toml"), format!("binding_version = 2\n[principal_project]\n{binding}")).unwrap();
            fs::write(f.cwd.join(".brain-notes.toml"), "# intentionally not consulted\n").unwrap();
        } else {
            fs::write(f.cwd.join(".brain-notes.toml"), format!("binding_version = 1\n{binding}")).unwrap();
        }
        f
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_brain"));
        c.current_dir(&self.cwd).env("HOME", &self.home).env("USERPROFILE", &self.home);
        c
    }
    fn call(&self, endpoint: &str, op: &str, arguments: Value) -> Value {
        let id = Uuid::new_v4();
        let mut child = self.command().arg(endpoint).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let request = json!({"protocol_version":1,"request_id":id,"operation":op,"arguments":arguments});
        child.stdin.take().unwrap().write_all(&serde_json::to_vec(&request).unwrap()).unwrap();
        let response = child.wait_with_output().unwrap();
        assert!(response.status.success(), "{response:?}");
        let value: Value = serde_json::from_slice(&response.stdout).unwrap();
        assert_eq!(value["request_id"], id.to_string());
        value["result"].clone()
    }
    fn observed(&self) -> Value {
        let status = self.call("projects", "status", json!({}));
        assert_eq!(status["status"], "found", "{status}");
        let read = self.call("projects", "read", json!({"project_id":self.id}));
        assert_eq!(read["status"], "found", "{read}");
        let mut expected = status["expected"].as_object().unwrap().clone();
        expected.remove("memory").unwrap();
        json!({"expected_project":read["project"],"expected":expected,"expected_markdown":read["markdown"],"body":BODY,"human_approved":true})
    }
    fn edit(&self, args: Value) -> Value { self.call("projects", "update_body", args) }
}

#[test]
fn edits_only_body_and_preserves_legacy_filename_crlf_and_existing_memory() {
    for unified in [false, true] {
        let f = Fixture::new(unified, true);
        // Exercise a complete binding with Memory in the unified case.
        let memory = unified.then(|| {
            let saved = f.call("memory", "create", json!({"context":{"working_directory":f.cwd,"authorized_boundary":f.cwd},
                "title":"Continuidad", "sections":["Objetivo","Decisiones","Estado","Evidencia","Siguiente paso"], "summary":"Primer resultado útil."}));
            assert_eq!(saved["status"], "saved", "{saved}");
            let folder = f.vault.join("05-memorias").join(saved["memory_id"].as_str().unwrap());
            ["Memoria.md", "Historial.md"].map(|name| { let p = folder.join(name); let bytes = fs::read(&p).unwrap(); (p, bytes) })
        });
        let marker = if unified { ".brain.toml" } else { ".brain-notes.toml" };
        let marker = f.cwd.join(marker);
        let marker_before = fs::read(&marker).unwrap();
        let old = f.observed();
        assert_eq!(f.edit(old.clone()), json!({"status":"updated","project":old["expected_project"]}));
        assert_eq!(fs::read_to_string(&f.path).unwrap(), format!("{}{BODY}", f.header));
        assert_eq!(fs::read(&marker).unwrap(), marker_before);
        assert!(!f.path.parent().unwrap().join("notas").exists());
        if let Some(memory) = memory { for (p, before) in memory { assert_eq!(fs::read(p).unwrap(), before); } }
        assert_eq!(f.edit(old)["status"], "conflict");
        assert_eq!(f.edit(f.observed())["status"], "unchanged");
    }
}

#[test]
fn rejects_disapproval_read_only_other_project_bad_body_and_changed_scope_without_replacing() {
    let f = Fixture::new(true, true);
    let args = f.observed();
    let mut no = args.clone(); no["human_approved"] = json!(false);
    assert_eq!(f.edit(no)["status"], "denied");
    let mut wrong = args.clone(); wrong["expected_project"]["project_id"] = json!(Uuid::new_v4());
    assert_eq!(f.edit(wrong)["status"], "denied");
    let mut invalid = args.clone(); invalid["body"] = json!("## Summary\nSin estructura.\n");
    assert_eq!(f.edit(invalid)["status"], "invalid");
    let marker = f.cwd.join(".brain.toml");
    let read_only = fs::read_to_string(&marker).unwrap().replace("allow_write = true", "allow_write = false");
    fs::write(marker, read_only).unwrap();
    assert_eq!(f.edit(args)["status"], "conflict");
    assert_eq!(f.edit(f.observed())["status"], "denied");
    assert_eq!(fs::read_to_string(&f.path).unwrap(), f.header);
    assert!(!f.cwd.join(".brain.toml.lock").exists());
    assert!(!f.vault.join("01-proyectos/.brain-projects.lock").exists());
}
