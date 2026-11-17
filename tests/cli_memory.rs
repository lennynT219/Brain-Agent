use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use uuid::Uuid;

fn run(root: &Path, args: &[&str], input: &[u8]) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn init(root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .args(["init", "vault"])
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::create_dir(root.join("work")).unwrap();
}
fn request(root: &Path, n: u128, operation: &str, arguments: Value) -> Value {
    json!({"protocol_version":1,"request_id":Uuid::from_u128(n),"operation":operation,"vault":root.join("vault"),"arguments":arguments})
}
fn call(root: &Path, n: u128, operation: &str, arguments: Value) -> Value {
    run(
        root,
        &["memory"],
        &serde_json::to_vec(&request(root, n, operation, arguments)).unwrap(),
    )
}
fn context(root: &Path) -> Value {
    json!({"working_directory":root.join("work"),"authorized_boundary":root.join("work")})
}

#[test]
fn capabilities_needs_neither_vault_nor_profile_and_correlates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let input = json!({"protocol_version":1,"request_id":Uuid::from_u128(1),"operation":"capabilities","arguments":{}});
    let output = run(root, &["memory"], &serde_json::to_vec(&input).unwrap());
    assert_eq!(output["request_id"], input["request_id"]);
    assert_eq!(output["operation"], "capabilities");
    assert_eq!(output["result"]["status"], "found");
    assert_eq!(output["result"]["max_input_bytes"], 4_194_304);
    assert_eq!(output["result"]["max_output_bytes"], 4_194_304);
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}

#[test]
fn end_to_end_json_create_resolve_update_manual_review_and_approval() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    assert_eq!(
        call(root, 1, "resolve", context(root))["result"]["status"],
        "unbound"
    );
    assert!(!root.join("vault/05-memorias").exists());
    let created = call(
        root,
        2,
        "create",
        json!({"context":context(root),"title":"CLI context","sections":["Goal","Decision","In progress","Evidence","Next"]}),
    );
    assert_eq!(created["result"]["status"], "saved");
    let id = created["result"]["memory_id"].clone();
    let read = call(root, 3, "read", json!({"memory_id":id}));
    let accepted = read["result"]["accepted_markdown"].as_str().unwrap();
    let document = brain::memory::markdown::MemoryDocument::parse(
        serde_json::from_value(id.clone()).unwrap(),
        accepted,
    )
    .unwrap();
    let expected = json!({"revision":1,"accepted_markdown":accepted});
    let body = document.body().replace("In progress", "Done");
    let updated = call(
        root,
        4,
        "update",
        json!({"memory_id":id,"expected":expected,"body":body}),
    );
    assert_eq!(updated["result"]["status"], "saved");
    assert_eq!(updated["result"]["revision"], 2);
    let stale = call(
        root,
        5,
        "update",
        json!({"memory_id":id,"expected":expected,"body":body}),
    );
    assert_eq!(stale["result"]["status"], "conflict");
    let read = call(root, 6, "read", json!({"memory_id":id}));
    let old = read["result"]["accepted_markdown"].as_str().unwrap();
    let manual = old.replace("Done", "Human decision");
    let path = root
        .join("vault/05-memorias")
        .join(id.as_str().unwrap())
        .join("Memoria.md");
    fs::write(&path, &manual).unwrap();
    let reviewed = call(root, 7, "read", json!({"memory_id":id}));
    assert_eq!(reviewed["result"]["status"], "manual_review");
    let candidate = reviewed["result"]["approval_candidate"].clone();
    let approved = call(
        root,
        8,
        "accept_external",
        json!({"memory_id":id,"expected":{"revision":2,"accepted_markdown":old},"observed_markdown":manual,"approved_markdown":candidate,"human_approved":true}),
    );
    assert_eq!(approved["result"]["status"], "saved");
    assert_eq!(approved["result"]["revision"], 3);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        candidate.as_str().unwrap()
    );
    assert!(!root.join("home").exists());
}

#[test]
fn protocol_and_io_bounds_are_explicit_and_do_not_create_storage() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let malformed = run(root, &["memory"], b"{not json");
    assert_eq!(malformed["result"]["status"], "invalid");
    let mut input = request(root, 1, "read", json!({"memory_id":Uuid::from_u128(42)}));
    input["protocol_version"] = json!(99);
    let output = run(root, &["memory"], &serde_json::to_vec(&input).unwrap());
    assert_eq!(output["result"]["status"], "unsupported");
    assert_eq!(output["request_id"], input["request_id"]);
    assert_eq!(
        call(root, 2, "unknown", json!({}))["result"]["status"],
        "unsupported"
    );
    assert_eq!(
        call(
            root,
            3,
            "resolve",
            json!({"working_directory":root.join("work"),"authorized_boundary":root.join("work"),"extra":true})
        )["result"]["status"],
        "invalid"
    );
    let oversized = run(
        root,
        &["memory", "--max-input-bytes", "1024"],
        &vec![b' '; 1025],
    );
    assert_eq!(oversized["result"]["status"], "too_large");
    let long_operation = request(root, 19, &"x".repeat(3000), json!({}));
    let bounded = run(
        root,
        &["memory", "--max-output-bytes", "1024"],
        &serde_json::to_vec(&long_operation).unwrap(),
    );
    assert_eq!(bounded["result"]["status"], "invalid");
    assert_eq!(bounded["request_id"], long_operation["request_id"]);
    assert!(serde_json::to_vec(&bounded).unwrap().len() < 1024); // Leave one byte for LF.
    assert!(!root.join("vault/05-memorias").exists());
    let created = call(
        root,
        4,
        "create",
        json!({"context":context(root),"title":"Large transport fixture","sections":["a".repeat(2000),"b","c","d","e"]}),
    );
    let id = created["result"]["memory_id"].clone();
    assert_eq!(created["result"]["status"], "saved");
    let request = request(root, 5, "read", json!({"memory_id":id}));
    let output = run(
        root,
        &["memory", "--max-output-bytes", "1024"],
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(output["result"]["status"], "too_large");
    assert!(!output["result"]["required_bytes"].is_null());
    assert_eq!(
        call(root, 6, "read", json!({"memory_id":id}))["result"]["status"],
        "found"
    );
}

#[test]
fn missing_memory_and_unavailable_vault_are_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let id = Uuid::from_u128(45);
    assert_eq!(
        call(root, 1, "read", json!({"memory_id":id}))["result"]["status"],
        "not_found"
    );
    let mut input = request(root, 2, "read", json!({"memory_id":id}));
    input["vault"] = json!(root.join("nonexistent"));
    assert_eq!(
        run(root, &["memory"], &serde_json::to_vec(&input).unwrap())["result"]["status"],
        "unavailable"
    );
}

#[test]
fn cancelled_input_process_does_not_create_any_binding() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .args(["memory"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"protocol_version\":1,")
        .unwrap();
    child.kill().unwrap();
    let _ = child.wait().unwrap();
    assert!(!root.join("work/.brain.toml").exists());
    assert!(!root.join("vault/05-memorias").exists());
}

#[test]
fn init_and_config_preserve_existing_memory_files_and_binding() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let created = call(
        root,
        1,
        "create",
        json!({"context":context(root),"title":"Preserved","sections":["a","b","c","d","e"]}),
    );
    let id = created["result"]["memory_id"].as_str().unwrap();
    let dir = root.join("vault/05-memorias").join(id);
    let current = fs::read(dir.join("Memoria.md")).unwrap();
    let history = fs::read(dir.join("Historial.md")).unwrap();
    let marker = fs::read(root.join("work/.brain.toml")).unwrap();
    for args in [
        vec!["init", "vault"],
        vec!["config", "set-vault", "vault"],
        vec!["config", "show"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_brain"))
            .current_dir(root)
            .env("HOME", root.join("home"))
            .env("USERPROFILE", root.join("home"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read(dir.join("Memoria.md")).unwrap(), current);
    assert_eq!(fs::read(dir.join("Historial.md")).unwrap(), history);
    assert_eq!(fs::read(root.join("work/.brain.toml")).unwrap(), marker);
    let mut request = request(root, 2, "read", json!({"memory_id":id}));
    request.as_object_mut().unwrap().remove("vault");
    let output = run(root, &["memory"], &serde_json::to_vec(&request).unwrap());
    assert_eq!(output["result"]["status"], "found");
    assert_eq!(
        fs::read_dir(root.join("vault/05-memorias"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn lost_stdout_confirmation_does_not_mean_the_save_was_rolled_back() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let input = request(
        root,
        77,
        "create",
        json!({"context":context(root),"title":"Sensitive fixture title","sections":["a","b","c","d","e"]}),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .arg("memory")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // The host loses its receive channel before publication.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Sensitive fixture title"));
    let binding = brain::memory::binding::Binding::parse(
        &fs::read_to_string(root.join("work/.brain.toml")).unwrap(),
    )
    .unwrap();
    let state = brain::memory::store::Store::open(&root.join("vault"))
        .unwrap()
        .read(binding.memory_id)
        .unwrap();
    assert_eq!(state.status, "found");
    assert_eq!(state.revision, Some(1));
    assert_eq!(
        fs::read_dir(root.join("vault/05-memorias"))
            .unwrap()
            .count(),
        1
    );
}
