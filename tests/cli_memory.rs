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
fn section_edits_use_one_exact_baseline_and_publish_one_history_entry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let created = call(
        root, 150, "create",
        json!({"context":context(root),"title":"Ediciones","sections":["a","b","c","d","e"],"summary":"Inicio."}),
    );
    assert_eq!(created["result"]["status"], "saved");
    let id: Uuid = serde_json::from_value(created["result"]["memory_id"].clone()).unwrap();
    let read = || call(root, 151, "read", json!({"memory_id":id}))["result"].clone();
    let expected = |state: &Value| json!({"revision":state["revision"],"accepted_markdown":state["accepted_markdown"]});
    let link = brain::memory::history_link(id);
    let body = format!(
        "Preámbulo\n\n## Objective and scope\r\nÚltimo ááá; único\r\n\r\n## Current decisions\rDecision\r\r## Outcome status\núnico y BORRAR\n## Relevant evidence\r\n{link}\r\n\r\n## Next actions and open questions"
    );
    let seeded = call(root, 152, "update", json!({"memory_id":id,"expected":expected(&read()),"body":body}));
    assert_eq!(seeded["result"]["status"], "saved");
    let before = read();
    let baseline = expected(&before);
    let dir = root.join("vault/05-memorias").join(id.to_string());
    let files = || ["Memoria.md", "Historial.md", ".accepted.json"].map(|file| fs::read(dir.join(file)).unwrap());
    let original = files();
    let edit = |section: &str, old_text: &str, new_text: &str| {
        json!({"section":section,"oldText":old_text,"newText":new_text})
    };
    let objective = "Objective and scope";
    let outcome = "Outcome status";
    let exact = edit(objective, "único\r\n", "único nuevo\r\n");
    let run_edits = |n, base: &Value, edits: Value| {
        call(root, n, "update_sections", json!({"memory_id":id,"expected":base,"edits":edits,"summary":"Edición puntual."}))["result"].clone()
    };

    for (batch, reason) in [
        (json!([exact, edit(outcome, "ausente", "x")]), Some("EDIT_NOT_FOUND")),
        (json!([edit(objective, "áá", "x")]), Some("EDIT_AMBIGUOUS")),
        (json!([exact, edit(objective, "; único", "x")]), Some("EDITS_OVERLAP")),
        (json!([edit(objective, "scope\r\nÚltimo", "x")]), Some("EDIT_NOT_FOUND")),
        (json!([edit(objective, "\nÚltimo", "x")]), Some("EDIT_NOT_FOUND")),
        (json!([edit(objective, "único\r\n\r\n## Current decisions", "x")]), Some("EDIT_NOT_FOUND")),
        (json!([edit("Next actions and open questions", "\n", "x")]), Some("EDIT_NOT_FOUND")),
        (json!([edit("Relevant evidence", &link, &format!("`{link}`"))]), None),
        (json!([edit("Current decisions", "Decision", "\nDecision")]), None),
        (json!([edit(objective, "Último", "## Extra\r\n")]), None),
        (json!([edit(objective, "", "x")]), None),
        (json!([edit("Unknown section", "único", "x")]), None),
        (json!([{"section":objective,"oldText":"Último","newText":"x","unknown":true}]), None),
        (json!([]), None),
        (json!(vec![exact.clone(); 33]), None),
    ] {
        let rejected = run_edits(153, &baseline, batch);
        assert_eq!(rejected["status"], "invalid", "{rejected}");
        assert_eq!(rejected["reason"].as_str(), reason, "{rejected}");
        if reason.is_some() {
            assert_eq!(rejected["message"], "document or arguments failed validation");
        }
        assert_eq!(files(), original);
    }
    let common = json!({"memory_id":id,"expected":baseline,"summary":"Prueba de esquema."});
    for fields in [json!({}), json!({"changes":null,"edits":[exact]}), json!({"changes":{(objective):"otro"},"edits":[exact]})] {
        let mut args = common.clone();
        args.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        assert_eq!(call(root, 154, "update_sections", args)["result"]["status"], "invalid");
        assert_eq!(files(), original);
    }
    let saved = run_edits(155, &baseline, json!([
        edit(outcome, " BORRAR", ""),
        edit(objective, "Último", "único\r\n"),
        exact,
    ]));
    assert_eq!(saved["status"], "saved");
    assert_eq!(saved["revision"], 3);
    let after = read();
    assert_eq!(after["accepted_markdown"].as_str().unwrap(),
        before["accepted_markdown"].as_str().unwrap()
            .replace("memory_revision: 2", "memory_revision: 3")
            .replace("Último ááá; único\r\n", "único\r\n ááá; único nuevo\r\n")
            .replace("y BORRAR", "y"));
    let published = files();
    let entry = published[1].strip_prefix(original[1].as_slice()).unwrap();
    let entry = std::str::from_utf8(entry).unwrap();
    assert_eq!(entry.matches("\n- ").count(), 1);
    assert!(entry.contains("Edición puntual."));
    assert_eq!(run_edits(156, &baseline, json!([exact]))["status"], "conflict");
    let wrong_bytes = json!({"revision":3,"accepted_markdown":before["accepted_markdown"]});
    assert_eq!(run_edits(157, &wrong_bytes, json!([exact]))["status"], "conflict");
    let no_op = run_edits(158, &expected(&after), json!([edit(outcome, "y", "y")]));
    assert_eq!(no_op["status"], "unchanged");
    assert_eq!(no_op["revision"], 3);
    assert_eq!(files(), published);

    let changed = call(root, 159, "update_sections", json!({"memory_id":id,"expected":expected(&after),"changes":{(outcome):"Cerrado\n"},"summary":"Reemplazo completo."}));
    assert_eq!(changed["result"]["status"], "saved");
    assert!(read()["accepted_markdown"].as_str().unwrap().contains("## Outcome status\nCerrado\n"));
}

#[test]
fn whole_section_separator_precondition_is_changes_only_and_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let created = call(
        root, 160, "create",
        json!({"context":context(root),"title":"Separador","sections":["A","B","antes","D","E"],"summary":"Inicio."}),
    );
    assert_eq!(created["result"]["status"], "saved");
    let id: Uuid = serde_json::from_value(created["result"]["memory_id"].clone()).unwrap();
    let read = || call(root, 161, "read", json!({"memory_id":id}))["result"].clone();
    let expected = |state: &Value| {
        json!({"revision":state["revision"],"accepted_markdown":state["accepted_markdown"]})
    };
    let initial = read();
    let dir = root.join("vault/05-memorias").join(id.to_string());
    let files = || {
        ["Memoria.md", "Historial.md", ".accepted.json"]
            .map(|file| fs::read(dir.join(file)).unwrap())
    };
    let original = files();
    let call_update = |n, state: &Value, fields: Value| {
        let mut args = json!({"memory_id":id,"expected":expected(state),"summary":"Cambia sección."});
        args.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        call(root, n, "update_sections", args)["result"].clone()
    };
    for field in [json!(false), Value::Null, json!("true"), json!(1)] {
        let rejected = call_update(162, &initial, json!({"changes":{"Outcome status":"después"},"require_preserved_separator":field}));
        assert_eq!(rejected["status"], "invalid");
    }
    let rejected = call_update(163, &initial, json!({"edits":[{"section":"Outcome status","oldText":"antes","newText":"después"}],"require_preserved_separator":true}));
    assert_eq!(rejected["status"], "invalid");
    assert_eq!(files(), original);

    let saved = call_update(164, &initial, json!({"changes":{"Outcome status":"después"},"require_preserved_separator":true}));
    assert_eq!(saved["status"], "saved");
    let next = read();
    assert_eq!(next["accepted_markdown"].as_str().unwrap(), initial["accepted_markdown"].as_str().unwrap().replace("memory_revision: 1", "memory_revision: 2").replace("antes\n\n", "después\n\n"));
    let published = files();
    let entry = published[1].strip_prefix(original[1].as_slice()).unwrap();
    assert_eq!(std::str::from_utf8(entry).unwrap().matches("\n- ").count(), 1);
    assert_eq!(call_update(165, &next, json!({"changes":{"Outcome status":"después"}}))["status"], "unchanged");
    assert_eq!(files(), published);

    let markdown = next["accepted_markdown"].as_str().unwrap();
    let doc = brain::memory::markdown::MemoryDocument::parse(id, markdown).unwrap();
    let mixed = doc.body().replace("## Outcome status\ndespués\n\n", "## Outcome status\rdespués\n\n");
    let seeded = call(root, 166, "update", json!({"memory_id":id,"expected":expected(&next),"body":mixed}));
    assert_eq!(seeded["result"]["status"], "saved");
    let mixed = read();
    let mixed_files = files();
    let rejected = call_update(167, &mixed, json!({"changes":{"Outcome status":"","Current decisions":"otro"},"require_preserved_separator":true}));
    assert_eq!(rejected["status"], "invalid");
    assert_eq!(files(), mixed_files);
}

#[test]
fn unclosed_html_create_has_fixed_reason_without_persisted_identity() {
    for content in [
        "05-memorias/<UUID>/Memoria.md",
        "ss_<area>_<action>",
        "<span>literal",
        "<!-- literal without terminator",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init(root);
        let output = call(
            root,
            101,
            "create",
            json!({"context":context(root),"title":"Private title","sections":[content,"b","c","d","e"]}),
        );
        assert_eq!(output["protocol_version"], 1);
        assert_eq!(output["request_id"], json!(Uuid::from_u128(101)));
        assert_eq!(output["operation"], "create");
        assert_eq!(
            output["result"],
            json!({"status":"invalid","reason":"UNCLOSED_HTML","message":"document or arguments failed validation"})
        );
        assert!(!root.join("work/.brain.toml").exists());
        assert!(!root.join("work/.brain.pending.toml").exists());
        assert!(!root.join("vault/05-memorias").exists());
        assert_eq!(
            call(root, 102, "resolve", context(root))["result"]["status"],
            "unbound"
        );
    }
}

#[test]
fn literal_placeholders_and_h3_remain_valid_without_normalization() {
    for content in [
        "`05-memorias/<UUID>/Memoria.md` and `ss_<area>_<action>`",
        r"05-memorias/\<UUID\>/Memoria.md and ss_\<area\>_\<action\>",
        "### Detail\nOrdinary content",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        init(root);
        let saved = call(
            root,
            103,
            "create",
            json!({"context":context(root),"title":"Valid syntax","sections":[content,"b","c","d","e"]}),
        );
        assert_eq!(saved["result"]["status"], "saved");
        let read = call(
            root,
            104,
            "read",
            json!({"memory_id":saved["result"]["memory_id"]}),
        );
        assert_eq!(read["result"]["status"], "found");
        assert!(
            read["result"]["accepted_markdown"]
                .as_str()
                .unwrap()
                .contains(content)
        );
    }
}

#[test]
fn unclosed_html_update_preserves_baseline_and_other_rejections_stay_generic() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init(root);
    let saved = call(
        root,
        105,
        "create",
        json!({"context":context(root),"title":"Baseline","sections":["a","b","c","d","e"]}),
    );
    assert_eq!(saved["result"]["status"], "saved");
    let id = saved["result"]["memory_id"].clone();
    let read = call(root, 106, "read", json!({"memory_id":id}));
    let accepted = read["result"]["accepted_markdown"].as_str().unwrap();
    let document = brain::memory::markdown::MemoryDocument::parse(
        serde_json::from_value(id.clone()).unwrap(),
        accepted,
    )
    .unwrap();
    let dir = root.join("vault/05-memorias").join(id.as_str().unwrap());
    let current = fs::read(dir.join("Memoria.md")).unwrap();
    let history = fs::read(dir.join("Historial.md")).unwrap();
    let marker = fs::read(root.join("work/.brain.toml")).unwrap();
    for (suffix, reason) in [
        ("05-memorias/<UUID>/Memoria.md", Some("UNCLOSED_HTML")),
        ("<span>literal", Some("UNCLOSED_HTML")),
        ("<!-- literal without terminator", Some("UNCLOSED_HTML")),
        ("```text\nunclosed fence", None),
        ("## Unexpected section", None),
    ] {
        let output = call(
            root,
            107,
            "update",
            json!({"memory_id":id,"expected":{"revision":1,"accepted_markdown":accepted},"body":format!("{}\n{suffix}\n", document.body())}),
        );
        let mut expected = json!({"status":"invalid","message":"document or arguments failed validation"});
        if let Some(reason) = reason {
            expected["reason"] = json!(reason);
        }
        assert_eq!(output["result"], expected);
        assert_eq!(output["operation"], "update");
        assert_eq!(output["request_id"], json!(Uuid::from_u128(107)));
        assert_eq!(fs::read(dir.join("Memoria.md")).unwrap(), current);
        assert_eq!(fs::read(dir.join("Historial.md")).unwrap(), history);
        assert_eq!(fs::read(root.join("work/.brain.toml")).unwrap(), marker);
        assert!(!root.join("work/.brain.pending.toml").exists());
        assert!(!dir.join(".pending.json").exists());
    }
    let after = call(root, 108, "read", json!({"memory_id":id}));
    assert_eq!(after["result"]["status"], "found");
    assert_eq!(after["result"]["revision"], 1);
    assert_eq!(after["result"]["accepted_markdown"], accepted);
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
