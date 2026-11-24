use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;
use uuid::Uuid;

const PROJECT: &str = "b341b410-5200-46f5-9ea3-e34caa551005";
const OTHER_PROJECT: &str = "b341b410-5200-46f5-9ea3-e34caa551006";
const NOTE: &str = "b341b410-5200-46f5-9ea3-e34caa551007";
const OTHER_NOTE: &str = "b341b410-5200-46f5-9ea3-e34caa551008";
const MARKER: &str = ".brain-notes.toml";
const LOCK: &str = ".brain-notes.lock";

fn document(kind: &str, id: &str, body: &str) -> String {
    format!(
        "---\nbrain_version: 1\nbrain_type: {kind}\nbrain_id: '{id}'\ntitle: Título\n---\n{body}"
    )
}

fn verify_process(output: &Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

struct Fixture {
    _temp: TempDir,
    vault: PathBuf,
    home: PathBuf,
    context: PathBuf,
    project: PathBuf,
    notes: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let home = temp.path().join("home");
        let context = temp.path().join("context");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&context).unwrap();
        let project = vault.join("01-proyectos/A");
        let notes = project.join("notas");
        let fixture = Self {
            _temp: temp,
            vault,
            home,
            context,
            project,
            notes,
        };
        let output = fixture
            .command(&fixture.context)
            .arg("init")
            .arg(&fixture.vault)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let output = fixture
            .command(&fixture.context)
            .args(["config", "set-vault"])
            .arg(&fixture.vault)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        fs::create_dir_all(&fixture.notes).unwrap();
        fs::write(
            fixture.project.join("Proyecto.md"),
            document("project", PROJECT, "# Proyecto\n"),
        )
        .unwrap();
        fs::write(
            fixture.notes.join("Original.md"),
            document("note", NOTE, "Original\n"),
        )
        .unwrap();
        fixture
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_brain"));
        command
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home);
        command
    }

    fn bind(&self, permission: Option<bool>) {
        let mut binding = format!(
            "binding_version = 1\nproject_id = '{PROJECT}'\nproject_path = '01-proyectos/A/Proyecto.md'\n"
        );
        if let Some(allowed) = permission {
            binding.push_str(&format!("allow_write = {allowed}\n"));
        }
        fs::write(self.context.join(MARKER), binding).unwrap();
    }

    fn exchange(&self, cwd: &Path, bytes: &[u8]) -> Value {
        let mut child = self
            .command(cwd)
            .arg("notes")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        verify_process(&output);
        assert_eq!(output.stdout.iter().filter(|&&b| b == b'\n').count(), 1);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn call_at(&self, cwd: &Path, operation: &str, arguments: Value) -> Value {
        let id = Uuid::new_v4().to_string();
        let request = json!({"protocol_version": 1, "request_id": id, "operation": operation, "arguments": arguments});
        let envelope = self.exchange(cwd, &serde_json::to_vec(&request).unwrap());
        assert_eq!(envelope["protocol_version"], 1);
        assert_eq!(envelope["request_id"], id);
        envelope["result"].clone()
    }

    fn call(&self, operation: &str, arguments: Value) -> Value {
        self.call_at(&self.context, operation, arguments)
    }

    fn read(&self, note_id: &str) -> Value {
        self.call("read", json!({"note_id": note_id}))
    }

    fn update(&self, note_id: &str, expected: &Value, body: &str) -> Value {
        self.call("update", json!({"project_id": PROJECT, "note_id": note_id, "expected_markdown": expected, "body": body}))
    }

    fn create(&self, title: &str, body: &str) -> Value {
        self.call(
            "create",
            json!({"project_id": PROJECT, "title": title, "body": body}),
        )
    }
}

#[test]
fn reads_only_direct_notes_in_exactly_bound_project() {
    let f = Fixture::new();
    let adjacent = f.vault.join("01-proyectos/A-extra/notas");
    let nested = f.notes.join("nested");
    fs::create_dir_all(&adjacent).unwrap();
    fs::write(
        adjacent.parent().unwrap().join("Proyecto.md"),
        document("project", OTHER_PROJECT, ""),
    )
    .unwrap();
    fs::create_dir(&nested).unwrap();
    for path in [
        adjacent.join("Original.md"),
        nested.join("Original.md"),
        f.project.join("Top.md"),
        f.vault.join("03-recursos/Original.md"),
    ] {
        fs::write(
            path,
            document("note", OTHER_NOTE, "SECRETO FUERA DEL ALCANCE\n"),
        )
        .unwrap();
    }
    fs::write(f.notes.join("plain.md"), "# texto sin metadatos\n").unwrap();
    fs::write(
        f.notes.join("tarea.md"),
        document("task", OTHER_NOTE, "No es Note\n"),
    )
    .unwrap();
    fs::write(
        f.notes.join("fake.txt"),
        document("note", OTHER_NOTE, "No es Markdown\n"),
    )
    .unwrap();
    fs::create_dir(f.notes.join("carpeta.md")).unwrap();
    // A valid Memory-only v1 marker is not a principal Project binding.
    fs::write(f.context.join(".brain.toml"), format!("binding_version=1\nmemory_id='{OTHER_PROJECT}'\n")).unwrap();
    assert_eq!(f.call("list", json!({}))["status"], "unbound");
    f.bind(None);
    let nested_context = f.context.join("sub");
    fs::create_dir(&nested_context).unwrap();
    assert_eq!(
        f.call_at(&nested_context, "list", json!({}))["status"],
        "unbound"
    );
    assert_eq!(
        f.call("list", json!({})),
        json!({"status":"listed","project_id":PROJECT,"notes":[{"note_id": NOTE,"title":"Título"}]})
    );
    let denied_read = f.read(OTHER_NOTE);
    assert_eq!(denied_read["status"], "not_found");
    assert!(!denied_read.to_string().contains("SECRETO"));
    assert_eq!(
        f.read(NOTE),
        json!({"status":"found","project_id":PROJECT,"note_id":NOTE,"title":"Título","markdown":document("note",NOTE,"Original\n"),"body":"Original\n"})
    );
    assert!(!f.notes.join(LOCK).exists());
    assert_eq!(
        f.call(
            "read",
            json!({"note_id": NOTE,"path":"03-recursos/Original.md"})
        )["status"],
        "invalid"
    );
}

#[test]
fn requires_write_permission_and_project_id_and_rejects_stale_bytes() {
    let f = Fixture::new();
    let path = f.notes.join("Original.md");
    let raw = document("note", NOTE, "Original\n")
        .replace(
            "title: Título\n",
            "# comentario humano\ntitle: Título\ncustom: conservar\n",
        )
        .replace('\n', "\r\n");
    fs::write(&path, &raw).unwrap();
    f.bind(None);
    let first = f.read(NOTE);
    assert_eq!(first["markdown"], raw);
    assert_eq!(f.create("Otro", "body")["status"], "denied");
    assert_eq!(
        f.update(NOTE, &first["markdown"], "body")["status"],
        "denied"
    );
    assert!(!f.notes.join(LOCK).exists());
    f.bind(Some(true));
    assert_eq!(
        f.call(
            "create",
            json!({"project_id":OTHER_PROJECT,"title":"Otro","body":"body"})
        )["status"],
        "conflict"
    );
    let manual = raw.replace("Original", "Edición humana");
    fs::write(&path, &manual).unwrap();
    assert_eq!(
        f.update(NOTE, &first["markdown"], "stale")["status"],
        "conflict"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), manual);
    assert_eq!(
        f.update(OTHER_NOTE, &first["markdown"], "no")["status"],
        "not_found"
    );
    let new = f.read(NOTE);
    assert_eq!(
        f.update(NOTE, &new["markdown"], "Cuerpo nuevo\n")["status"],
        "updated"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        raw.replace("Original\r\n", "Cuerpo nuevo\n")
    );
    let updated = f.read(NOTE);
    assert_eq!(updated["body"], "Cuerpo nuevo\n");
    assert_eq!(
        f.update(NOTE, &updated["markdown"], "Cuerpo nuevo\n")["status"],
        "unchanged"
    );
    let lock = File::options()
        .write(true)
        .open(f.notes.join(LOCK))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(f.create("Bloqueada", "")["status"], "busy");
    drop(lock);
    let body = "# Cuerpo\r\n\r\n[[Referente]]\r\n";
    let created = f.call("create", json!({"project_id": PROJECT, "title":"Título de la nota", "body":body,
        "require_named_link": true}));
    assert_eq!(created["status"], "created");
    let new_id = created["note_id"].as_str().unwrap();
    assert!(!Uuid::parse_str(new_id).unwrap().is_nil());
    let new_path = f.notes.join("Título de la nota.md");
    let raw = fs::read_to_string(&new_path).unwrap();
    assert!(!f.notes.join(format!("{new_id}.md")).exists());
    let prefix = raw.strip_suffix(body).unwrap();
    assert!(prefix.contains("\nproject_link: \"[[01-proyectos/A/Proyecto]]\"\n---\n"));
    let found = f.read(new_id);
    assert_eq!(found["body"], body);
    assert_eq!(found["title"], "Título de la nota");
    assert_eq!(found["markdown"], raw);
    assert_eq!(f.update(new_id, &found["markdown"], "Cambió el cuerpo\n")["status"], "updated");
    assert_eq!(fs::read_to_string(new_path).unwrap(), format!("{prefix}Cambió el cuerpo\n"));
    assert_eq!(
        f.call("list", json!({}))["notes"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn rename_preserves_exact_other_bytes_and_never_overwrites_destination() {
    let f = Fixture::new();
    let source = f.notes.join("Original.md");
    let baseline = document("note", NOTE, "texto\r\n[[Otro]]\n").replace(
        "title: Título\n",
        "# keep this\r\ntitle: Título  \r\nproject_link: \"[[01-proyectos/A/Proyecto]]\"\r\nextra: keep\r\n",
    );
    fs::write(&source, &baseline).unwrap();
    let rename = |expected: &str, title: &str| f.call("rename", json!({
        "project_id":PROJECT,"note_id":NOTE,"expected_markdown":expected,"title":title
    }));
    f.bind(Some(false));
    assert_eq!(rename(&baseline, "Nuevo")["status"], "denied");
    f.bind(Some(true));
    assert_eq!(rename(&baseline, "Título")["status"], "unchanged");
    assert_eq!(rename("stale", "Nuevo")["status"], "conflict");
    assert_eq!(rename(&baseline, "../Nuevo")["status"], "invalid");
    let target = f.notes.join("Nuevo.md");
    fs::write(&target, "unmanaged").unwrap();
    assert_eq!(rename(&baseline, "Nuevo")["status"], "conflict");
    assert_eq!(fs::read_to_string(&target).unwrap(), "unmanaged");
    assert_eq!(fs::read_to_string(&source).unwrap(), baseline);
    fs::remove_file(&target).unwrap();
    let receipt = rename(&baseline, "Nuevo");
    assert_eq!(receipt, json!({"status":"renamed","project_id":PROJECT,"note_id":NOTE,"title":"Nuevo"}));
    assert!(!source.exists());
    let next = baseline.replace("title: Título  \r\n", "title: \"Nuevo\"  \r\n");
    assert_eq!(fs::read_to_string(&target).unwrap(), next);
    assert_eq!(f.read(NOTE)["markdown"], next);
    assert_eq!(rename(&next, "Nuevo")["status"], "unchanged");
    assert_eq!(f.call("inspect_rename", json!({"project_id":PROJECT}))["status"], "no_intent");
}

#[test]
fn retained_rename_intent_fences_operations_without_recovery_writes() {
    let f = Fixture::new();
    f.bind(Some(true));
    let source = f.notes.join("Original.md");
    let target = f.notes.join("Nuevo.md");
    let before = fs::read_to_string(&source).unwrap();
    let after = before.replace("title: Título", "title: Nuevo");
    File::create(f.notes.join(LOCK)).unwrap();
    let pending = f.notes.join(".brain-note-rename.json");
    let intent = json!({"version":1,"project_id":PROJECT,"note_id":NOTE,"source":"Original.md","destination":"Nuevo.md","before":before,"after":after}).to_string();
    fs::write(&pending, &intent).unwrap();
    let inspect = || f.call("inspect_rename", json!({"project_id":PROJECT}));
    let state = inspect();
    assert_eq!(state["status"], "partial");
    assert_eq!(state["source_state"], "before");
    assert_eq!(state["destination_state"], "absent");
    fs::write(&target, &after).unwrap();
    assert_eq!(inspect()["destination_state"], "after");
    assert_eq!(f.read(NOTE)["status"], "partial");
    assert_eq!(f.call("list", json!({"project_id":PROJECT}))["status"], "partial");
    assert_eq!(f.create("Otra", "body")["status"], "partial");
    assert_eq!(f.call("rename", json!({"project_id":PROJECT,"note_id":NOTE,"expected_markdown":before,"title":"Nuevo"}))["status"], "partial");
    assert_eq!(fs::read_to_string(&source).unwrap(), before);
    assert_eq!(fs::read_to_string(&target).unwrap(), after);
    assert_eq!(fs::read_to_string(&pending).unwrap(), intent);
    fs::remove_file(source).unwrap();
    assert_eq!(inspect()["source_state"], "absent");
    assert_eq!(inspect()["destination_state"], "after");
}

#[test]
fn named_creation_rejects_invalid_titles_and_collisions_without_replacing_notes() {
    let f = Fixture::new();
    f.bind(Some(true));
    for title in ["Título: no", "../ruta", "LPT¹", "CON .variante", ".oculta", "Nombre.", " Nombre", "Nombre ", &"á".repeat(127)] {
        assert_eq!(f.create(title, "body")["status"], "invalid", "{title}");
    }
    assert!(!f.notes.join(LOCK).exists());
    let legacy_file = f.notes.join("Original.md");
    let legacy = fs::read(&legacy_file).unwrap();
    assert_eq!(f.create("TÍTULO", "duplicate legacy title")["status"], "conflict");
    let unmanaged = f.notes.join("Naming conventions.md");
    fs::write(&unmanaged, "# Contenido no gestionado\n").unwrap();
    assert_eq!(f.create("Naming conventions", "avoid replacement")["status"], "conflict");
    assert_eq!(fs::read(&legacy_file).unwrap(), legacy);
    assert_eq!(fs::read_to_string(unmanaged).unwrap(), "# Contenido no gestionado\n");
    assert_eq!(f.call("list", json!({}))["notes"].as_array().unwrap().len(), 1);
}

#[test]
fn fails_closed_on_project_or_note_identity_ambiguity() {
    let f = Fixture::new();
    f.bind(Some(true));
    let second_project = f.project.join("Segundo.md");
    fs::write(&second_project, document("project", OTHER_PROJECT, "")).unwrap();
    assert_eq!(f.call("list", json!({}))["status"], "unavailable");
    assert_eq!(f.create("Otro", "-")["status"], "unavailable");
    assert!(!f.notes.join(LOCK).exists());
    fs::remove_file(second_project).unwrap();
    fs::write(
        f.notes.join("Duplicada.md"),
        document("note", NOTE, "Duplicada"),
    )
    .unwrap();
    assert_eq!(f.read(NOTE)["status"], "conflict");
    assert_eq!(f.create("Otro", "-")["status"], "conflict");
    assert_eq!(f.call("list", json!({}))["status"], "conflict");
    fs::write(f.context.join(MARKER), format!("binding_version = 1\nproject_id = '{PROJECT}'\nproject_path = '01-proyectos/../Proyecto.md'\n")).unwrap();
    assert_eq!(f.call("list", json!({}))["status"], "unavailable");
}

#[cfg(unix)]
#[test]
fn refuses_symlinks_and_hardlinks() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.bind(Some(true));
    let marker = f.context.join(MARKER);
    let outside_binding = f.context.join("other_binding");
    fs::rename(&marker, &outside_binding).unwrap();
    symlink(&outside_binding, &marker).unwrap();
    assert_eq!(f.call("list", json!({}))["status"], "unavailable");
    fs::remove_file(&marker).unwrap();
    f.bind(Some(true));
    let secret = f.vault.join("03-recursos/Secreta.md");
    fs::write(&secret, document("note", OTHER_NOTE, "secreta")).unwrap();
    let alias = f.notes.join("Alias.md");
    symlink(&secret, &alias).unwrap();
    assert_eq!(f.read(OTHER_NOTE)["status"], "unavailable");
    fs::remove_file(&alias).unwrap();
    fs::hard_link(&secret, &alias).unwrap();
    assert_eq!(f.read(OTHER_NOTE)["status"], "unavailable");
    fs::remove_file(&alias).unwrap();
    let relocated = f.project.join("other-notes");
    fs::rename(&f.notes, &relocated).unwrap();
    symlink(&relocated, &f.notes).unwrap();
    assert_eq!(f.create("Otro", "-")["status"], "unavailable");
    assert!(!relocated.join(LOCK).exists());
}

#[test]
fn bounds_wire_transfers_without_truncating_or_publishing() {
    let f = Fixture::new();
    f.bind(Some(true));
    let oversized = f.exchange(&f.context, &vec![b' '; 4 * 1024 * 1024 + 1]);
    assert_eq!(oversized["result"]["status"], "input_limit");
    assert!(oversized["request_id"].is_null());
    let invalid = f.exchange(
        &f.context,
        br#"{"protocol_version":1,"vault":"/tmp","arguments":{}}"#,
    );
    assert_eq!(invalid["result"]["status"], "invalid");
    let hostile_read = document("note", NOTE, &"\0".repeat(400_000));
    fs::write(f.notes.join("Original.md"), &hostile_read).unwrap();
    assert_eq!(f.read(NOTE)["status"], "output_limit");
    assert_eq!(
        f.create("Otra", &"a".repeat(512 * 1024))["status"],
        "invalid"
    );
    assert_eq!(
        f.call("list", json!({}))["notes"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        fs::read_to_string(f.notes.join("Original.md")).unwrap(),
        hostile_read
    );
}
