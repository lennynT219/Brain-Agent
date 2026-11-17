//! R1/R2/R3 regression fixtures only; no real vault or working marker.
use brain::memory::{
    CURRENT_SECTIONS,
    binding::Binding,
    history::{AcceptedSnapshot, History},
    history_link,
    markdown::{MemoryDocument, ProjectAssociation, create_document},
    store::{Context, Create, Expected, Fault, ReplaceMissing, Store},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};
use uuid::Uuid;

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn setup() -> (tempfile::TempDir, PathBuf, Context) {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    for dir in [
        "00-limbo",
        "01-proyectos",
        "02-areas",
        "03-recursos",
        "04-archivo",
        "99-sistema/plantillas",
        "99-sistema/documentacion",
    ] {
        fs::create_dir_all(vault.join(dir)).unwrap();
    }
    let work = temp.path().join("work");
    fs::create_dir(&work).unwrap();
    let ctx = Context {
        working_directory: work.clone(),
        authorized_boundary: work,
    };
    (temp, vault, ctx)
}
fn create(ctx: &Context) -> Create {
    Create {
        context: ctx.clone(),
        title: "Context".into(),
        sections: std::array::from_fn(|_| "Evidence".into()),
        project: None,
        memory_subfolders: vec![],
    }
}
fn file(vault: &Path, id: Uuid, name: &str) -> PathBuf {
    vault.join("05-memorias").join(id.to_string()).join(name)
}
fn marker(id: Uuid) -> String {
    format!(
        "# exact old marker\r\nbinding_version = 1\r\n\"memory_id\" = '{id}' # retain comment\r\nmemory_subfolders = [ 'child', 'nested/child' ]\r\n"
    )
}
fn replacement(ctx: &Context, id: Uuid, raw: &str) -> ReplaceMissing {
    ReplaceMissing {
        context: ctx.clone(),
        binding_directory: ctx.working_directory.clone(),
        expected_memory_id: id,
        expected_marker: raw.into(),
        human_approved: true,
        title: "Explicit new context".into(),
        sections: std::array::from_fn(|_| "Replacement proposal".into()),
        project: None,
    }
}
fn bind_missing(ctx: &Context, id: Uuid) -> String {
    let raw = marker(id);
    fs::write(ctx.working_directory.join(".brain.toml"), &raw).unwrap();
    raw
}
fn baseline(store: &Store, id: Uuid) -> Expected {
    let state = store.read(id).unwrap();
    assert_eq!(state.status, "found");
    Expected {
        revision: state.revision.unwrap(),
        accepted_markdown: state.accepted_markdown.unwrap(),
    }
}

fn inactive(link: &str) -> Vec<String> {
    vec![
        format!("```text\n{link}\n```"),
        format!("~~~~text\n{link}\n~~~~"),
        format!("    {link}"),
        format!("\t{link}"),
        format!(" \t{link}"),
        format!("  \t{link}"),
        format!("   \t{link}"),
        format!("`{link}`"),
        format!("``literal {link} literal``"),
        format!("prefix <!-- {link} --> suffix"),
        format!("prefix <!--\n{link}\n--> suffix"),
        format!("\\{link}"),
        format!("<code>{link}</code>"),
        format!("<span title='{link}'>literal</span>"),
        format!("<div><!-- comment > </div> --> {link}</div>"),
        format!("[example]({link})"),
        format!("[example](<foo){link}>)"),
        format!("[ref]: {link}"),
        format!("[{link}]"),
        link.replace("[[", "&#91;&#91;"),
    ]
}

#[test]
fn r1_required_history_and_project_links_must_be_active_not_examples() {
    let id = uid(1);
    let project = ProjectAssociation {
        brain_project: uid(2),
        path: "01-proyectos/P.md".into(),
        label: "P".into(),
    };
    let body = CURRENT_SECTIONS
        .iter()
        .map(|name| format!("## {name}\n\n"))
        .collect::<String>()
        // Separate paragraphs: otherwise four-space indentation of the Project
        // line is a genuine paragraph continuation, not an indented code block.
        + &format!("{}\n\n{}\n", history_link(id), project.link());
    let valid = create_document(id, "Context", &body, Some(&project)).unwrap();
    for link in [history_link(id), project.link()] {
        for literal in inactive(&link) {
            let body = valid.body().replace(&link, &literal);
            assert!(
                valid.successor(&body).is_err(),
                "accepted literal-only relation: {literal}"
            );
            let body = valid.body().replace(&link, &format!("{literal}\n\n{link}"));
            let successor = valid.successor(&body).unwrap();
            assert_eq!(successor.body(), body);
            let crlf = successor.raw.replace('\n', "\r\n");
            assert_eq!(MemoryDocument::parse(id, &crlf).unwrap().raw, crlf);
        }
        // A literal escaped backslash does not escape the following genuine link.
        assert!(
            valid
                .successor(&valid.body().replace(&link, &format!("\\\\{link}")))
                .is_ok()
        );
    }
}

#[test]
fn r1_multiline_and_unclosed_html_tags_fail_closed_in_supported_markdown() {
    let id = uid(1);
    let body = CURRENT_SECTIONS
        .iter()
        .map(|name| format!("## {name}\n\n"))
        .collect::<String>()
        + &history_link(id);
    let valid = create_document(id, "Context", &body, None).unwrap();
    for literal in [
        format!("<span\ntitle=\"{}\">literal</span>", history_link(id)),
        format!("<span title=\"{}\"", history_link(id)),
    ] {
        assert!(
            valid
                .successor(&valid.body().replace(&history_link(id), &literal))
                .is_err()
        );
        // Unsupported tag syntax is rejected, never treated as active even with a second link.
        assert!(
            valid
                .successor(&format!("{}\n{}", valid.body(), literal))
                .is_err()
        );
    }
}

#[test]
fn r1_legacy_literal_only_accepted_history_is_preserved_reported_not_rewritten() {
    let (_temp, vault, ctx) = setup();
    let store = Store::open(&vault).unwrap();
    let id = store.create(uid(1), create(&ctx)).unwrap().memory_id;
    let before = baseline(&store, id);
    let raw = before.accepted_markdown.replace(
        &history_link(id),
        &format!("```text\n{}\n```", history_link(id)),
    );
    let mut history = History::new(id).unwrap();
    history
        .push(AcceptedSnapshot {
            revision: 1,
            request_id: uid(1),
            markdown: raw.clone(),
        })
        .unwrap();
    let bytes = history.encode().unwrap();
    fs::write(file(&vault, id, "Memoria.md"), &raw).unwrap();
    fs::write(file(&vault, id, "Historial.md"), &bytes).unwrap();
    let state = store.read(id).unwrap();
    assert_eq!(state.status, "partial");
    assert_eq!(state.reason.as_deref(), Some("history_document_invalid"));
    assert_eq!(
        store
            .update(id, uid(2), before, "ignored")
            .unwrap_err()
            .status,
        "partial"
    );
    assert_eq!(
        fs::read_to_string(file(&vault, id, "Memoria.md")).unwrap(),
        raw
    );
    assert_eq!(
        fs::read_to_string(file(&vault, id, "Historial.md")).unwrap(),
        bytes
    );
}

#[test]
fn r2_approved_missing_replacement_preserves_marker_bytes_children_and_copies() {
    let (temp, vault, ctx) = setup();
    let store = Store::open(&vault).unwrap();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let copy = temp.path().join("copy");
    fs::create_dir(&copy).unwrap();
    fs::write(copy.join(".brain.toml"), &before).unwrap();
    assert_eq!(
        store.create(uid(1), create(&ctx)).unwrap_err().status,
        "conflict"
    );
    let saved = store
        .replace_missing(uid(2), replacement(&ctx, old, &before))
        .unwrap();
    assert_eq!(saved.status, "saved");
    assert_eq!(saved.replaced_memory_id, Some(old));
    assert_ne!(saved.memory_id, old);
    assert_eq!(saved.revision, 1);
    let after = fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap();
    assert_eq!(
        after,
        before.replacen(&format!("'{old}'"), &format!("\"{}\"", saved.memory_id), 1)
    );
    assert_eq!(
        Binding::parse(&after).unwrap().memory_subfolders,
        vec!["child", "nested/child"]
    );
    assert_eq!(
        fs::read_to_string(copy.join(".brain.toml")).unwrap(),
        before
    );
    assert!(!vault.join("05-memorias").join(old.to_string()).exists());
    assert_eq!(store.read(saved.memory_id).unwrap().status, "found");
    let provenance: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(file(&vault, saved.memory_id, ".reassociation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["before_marker"], before);
    assert_eq!(provenance["marker"], after);
    assert_eq!(provenance["previous_memory_id"], old.to_string());
}

#[test]
fn r2_false_approval_stale_marker_identity_target_and_child_scope_are_rejected() {
    let (_temp, vault, ctx) = setup();
    let store = Store::open(&vault).unwrap();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let mut args = replacement(&ctx, old, &before);
    args.human_approved = false;
    assert_eq!(
        store.replace_missing(uid(1), args).unwrap_err().status,
        "manual_review"
    );
    assert!(!ctx.working_directory.join(".brain.toml.lock").exists());
    let mut args = replacement(&ctx, uid(901), &before);
    assert_eq!(
        store
            .replace_missing(uid(2), args.clone())
            .unwrap_err()
            .status,
        "conflict"
    );
    args.expected_memory_id = old;
    args.expected_marker = format!("{before}# stale\n");
    assert_eq!(
        store.replace_missing(uid(3), args).unwrap_err().status,
        "conflict"
    );
    let child = ctx.working_directory.join("child");
    fs::create_dir(&child).unwrap();
    let mut args = replacement(&ctx, old, &before);
    args.context.working_directory = child.clone();
    assert_eq!(
        store.replace_missing(uid(4), args).unwrap_err().status,
        "conflict"
    ); // explicit declaration is unbound, not parent's marker
    let mut args = replacement(&ctx, old, &before);
    args.binding_directory = child.clone();
    assert_eq!(
        store.replace_missing(uid(5), args).unwrap_err().status,
        "conflict"
    );
    let mut args = replacement(&ctx, old, &before);
    args.context = Context {
        working_directory: child.clone(),
        authorized_boundary: child,
    };
    assert_eq!(
        store.replace_missing(uid(6), args).unwrap_err().status,
        "conflict"
    );
    assert_eq!(
        fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap(),
        before
    );
    assert!(!vault.join("05-memorias").exists());
}

#[test]
fn r2_existing_partial_unavailable_and_busy_old_memory_are_not_notfound() {
    for kind in [
        "found",
        "partial",
        "corrupt",
        "unavailable",
        "empty",
        "busy",
    ] {
        let (_temp, vault, ctx) = setup();
        let store = Store::open(&vault).unwrap();
        let old;
        let mut lock = None;
        if matches!(kind, "found" | "partial" | "corrupt" | "busy") {
            old = store.create(uid(1), create(&ctx)).unwrap().memory_id;
            if kind == "partial" {
                fs::remove_file(file(&vault, old, "Historial.md")).unwrap();
            }
            if kind == "corrupt" {
                fs::write(file(&vault, old, "Historial.md"), [0xff]).unwrap();
            }
            if kind == "busy" {
                let file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(file(&vault, old, ".lock"))
                    .unwrap();
                file.try_lock().unwrap();
                lock = Some(file);
            }
        } else {
            old = uid(900);
            fs::create_dir(vault.join("05-memorias")).unwrap();
            if kind == "unavailable" {
                fs::write(
                    vault.join("05-memorias").join(old.to_string()),
                    "not a directory",
                )
                .unwrap();
            } else {
                fs::create_dir(vault.join("05-memorias").join(old.to_string())).unwrap();
                fs::write(file(&vault, old, ".lock"), "").unwrap();
            }
            bind_missing(&ctx, old);
        }
        let before = fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap();
        let result = store
            .replace_missing(uid(2), replacement(&ctx, old, &before))
            .unwrap_err();
        assert_eq!(
            result.status,
            match kind {
                "found" => "conflict",
                "unavailable" => "unavailable",
                "busy" => "busy",
                _ => "partial",
            },
            "{kind}"
        );
        assert_eq!(
            fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap(),
            before
        );
        assert!(!ctx.working_directory.join(".brain.pending.toml").exists());
        drop(lock);
    }
}

#[test]
fn r2_concurrent_replacement_admits_one_new_identity() {
    let (_temp, vault, ctx) = setup();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let barrier = Arc::new(Barrier::new(2));
    let tasks = (1..=2)
        .map(|n| {
            let vault = vault.clone();
            let args = replacement(&ctx, old, &before);
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&vault).unwrap();
                barrier.wait();
                store.replace_missing(uid(n), args)
            })
        })
        .collect::<Vec<_>>();
    let results = tasks
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| matches!(e.status.as_str(), "busy" | "conflict" | "partial"))
    );
    assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 1);
}

#[test]
fn r2_interrupted_replacements_recover_exact_intent_without_a_second_identity() {
    for fault in [
        Fault::AfterPending,
        Fault::AfterCurrent,
        Fault::AfterHistory,
        Fault::BeforeMarker,
        Fault::AfterMarker,
        Fault::AfterReservationCleanup,
    ] {
        let (_temp, vault, ctx) = setup();
        let old = uid(900);
        let before = bind_missing(&ctx, old);
        let args = replacement(&ctx, old, &before);
        let faulty = Store::open(&vault).unwrap().with_fault(fault);
        let error = faulty.replace_missing(uid(1), args.clone()).unwrap_err();
        assert_eq!(error.status, "partial");
        let id = error.memory_id.unwrap();
        let store = Store::open(&vault).unwrap();
        assert!(store.replace_missing(uid(2), args).is_err());
        let observed = store.inspect_recovery(id, Some(&ctx)).unwrap();
        let mut stale = observed.clone();
        stale.marker = Some("changed".into());
        assert_eq!(
            store
                .recover(id, Some(&ctx), stale, true)
                .unwrap_err()
                .status,
            "conflict"
        );
        let saved = store.recover(id, Some(&ctx), observed, true).unwrap();
        assert_eq!(saved.replaced_memory_id, Some(old));
        assert_eq!(saved.memory_id, id);
        assert_eq!(
            Binding::parse(&fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap())
                .unwrap()
                .memory_id,
            id
        );
        assert_eq!(store.read(id).unwrap().status, "found");
        assert!(!ctx.working_directory.join(".brain.pending.toml").exists());
        assert!(file(&vault, id, ".reassociation.json").exists());
        assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 1);
        assert!(!vault.join("05-memorias").join(old.to_string()).exists());
    }
}

#[test]
fn r2_marker_changed_or_removed_during_recovery_is_never_overwritten_or_restored() {
    for changed in [
        None,
        Some("same identity, changed bytes"),
        Some("third identity"),
    ] {
        let (_temp, vault, ctx) = setup();
        let old = uid(900);
        let before = bind_missing(&ctx, old);
        let error = Store::open(&vault)
            .unwrap()
            .with_fault(Fault::AfterHistory)
            .replace_missing(uid(1), replacement(&ctx, old, &before))
            .unwrap_err();
        let id = error.memory_id.unwrap();
        let marker = ctx.working_directory.join(".brain.toml");
        let actual = match changed {
            None => {
                fs::remove_file(&marker).unwrap();
                None
            }
            Some("third identity") => {
                let raw = marker_string(uid(901));
                fs::write(&marker, &raw).unwrap();
                Some(raw)
            }
            Some(_) => {
                let raw = format!("{before}# changed\n");
                fs::write(&marker, &raw).unwrap();
                Some(raw)
            }
        };
        let store = Store::open(&vault).unwrap();
        let observation = store.inspect_recovery(id, Some(&ctx)).unwrap();
        assert_eq!(
            store
                .recover(id, Some(&ctx), observation, true)
                .unwrap_err()
                .status,
            "conflict"
        );
        assert_eq!(fs::read_to_string(&marker).ok(), actual);
        assert!(file(&vault, id, ".pending.json").exists());
    }
}
fn marker_string(id: Uuid) -> String {
    format!("binding_version=1\nmemory_id='{id}'\n")
}

#[test]
fn r2_old_memory_reappearance_before_marker_publication_blocks_replacement() {
    let (_temp, vault, ctx) = setup();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let error = Store::open(&vault)
        .unwrap()
        .with_fault(Fault::AfterHistory)
        .replace_missing(uid(1), replacement(&ctx, old, &before))
        .unwrap_err();
    let new = error.memory_id.unwrap();
    fs::create_dir(vault.join("05-memorias").join(old.to_string())).unwrap();
    fs::write(file(&vault, old, ".lock"), "").unwrap();
    let raw = fs::read_to_string(file(&vault, new, "Memoria.md"))
        .unwrap()
        .replace(&new.to_string(), &old.to_string());
    MemoryDocument::parse(old, &raw).unwrap();
    let mut history = History::new(old).unwrap();
    history
        .push(AcceptedSnapshot {
            revision: 1,
            request_id: uid(99),
            markdown: raw.clone(),
        })
        .unwrap();
    fs::write(file(&vault, old, "Memoria.md"), &raw).unwrap();
    fs::write(file(&vault, old, "Historial.md"), history.encode().unwrap()).unwrap();
    let store = Store::open(&vault).unwrap();
    let observation = store.inspect_recovery(new, Some(&ctx)).unwrap();
    assert_eq!(
        store
            .recover(new, Some(&ctx), observation, true)
            .unwrap_err()
            .status,
        "conflict"
    );
    assert_eq!(
        fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(file(&vault, old, "Memoria.md")).unwrap(),
        raw
    );
}

#[test]
fn r2_preintent_replacement_failure_preserves_old_marker_and_reserved_new_id() {
    let (_temp, vault, ctx) = setup();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let args = replacement(&ctx, old, &before);
    let error = Store::open(&vault)
        .unwrap()
        .with_fault(Fault::AfterReservation)
        .replace_missing(uid(1), args.clone())
        .unwrap_err();
    let id = error.memory_id.unwrap();
    assert_ne!(id, old);
    let store = Store::open(&vault).unwrap();
    assert_eq!(store.resolve(&ctx).unwrap()["status"], "partial");
    assert_eq!(
        store.replace_missing(uid(2), args).unwrap_err().status,
        "partial"
    );
    assert_eq!(
        fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap(),
        before
    );
    assert!(!vault.join("05-memorias").exists());
    let observed = store.inspect_recovery(id, Some(&ctx)).unwrap();
    assert!(observed.pending.is_none());
    assert!(observed.reservation.unwrap().contains(&id.to_string()));
}

#[test]
fn r3_invalid_utf8_and_truncated_multibyte_history_are_partial_exact_and_never_repaired() {
    for bytes in [vec![0xff, 0x00, 0xfe], vec![b'#', b' ', 0xf0, 0x9f, 0xa6]] {
        let (_temp, vault, ctx) = setup();
        let store = Store::open(&vault).unwrap();
        let id = store.create(uid(1), create(&ctx)).unwrap().memory_id;
        let expected = baseline(&store, id);
        let current = fs::read(file(&vault, id, "Memoria.md")).unwrap();
        let marker = fs::read(ctx.working_directory.join(".brain.toml")).unwrap();
        fs::write(file(&vault, id, "Historial.md"), &bytes).unwrap();
        let state = store.read(id).unwrap();
        assert_eq!(state.status, "partial");
        assert_eq!(state.reason.as_deref(), Some("history_corrupt"));
        assert!(state.accepted_markdown.is_none());
        let failure = store.update(id, uid(2), expected, "ignored").unwrap_err();
        assert_eq!(failure.status, "partial");
        assert_eq!(failure.reason.as_deref(), Some("history_corrupt"));
        let observed = store.inspect_recovery(id, None).unwrap();
        assert!(observed.history_corrupt);
        assert_eq!(observed.history_bytes, Some(bytes.clone()));
        assert!(observed.history.is_none());
        let failure = store.recover(id, None, observed, true).unwrap_err();
        assert_eq!(failure.status, "partial");
        assert_eq!(failure.reason.as_deref(), Some("history_corrupt"));
        assert_eq!(fs::read(file(&vault, id, "Historial.md")).unwrap(), bytes);
        assert_eq!(fs::read(file(&vault, id, "Memoria.md")).unwrap(), current);
        assert_eq!(
            fs::read(ctx.working_directory.join(".brain.toml")).unwrap(),
            marker
        );
        assert!(!file(&vault, id, ".pending.json").exists());
    }
}

#[test]
fn r3_valid_utf8_corruption_missing_history_and_actual_io_stay_distinct() {
    for kind in ["utf8_corrupt", "missing", "directory"] {
        let (_temp, vault, ctx) = setup();
        let store = Store::open(&vault).unwrap();
        let id = store.create(uid(1), create(&ctx)).unwrap().memory_id;
        let target = file(&vault, id, "Historial.md");
        fs::remove_file(&target).unwrap();
        match kind {
            "utf8_corrupt" => fs::write(&target, "truncated valid UTF-8 🦀").unwrap(),
            "directory" => fs::create_dir(&target).unwrap(),
            _ => {}
        };
        if kind == "directory" {
            assert_eq!(store.read(id).unwrap_err().status, "unavailable");
            assert_eq!(
                store.inspect_recovery(id, None).unwrap_err().status,
                "unavailable"
            );
        } else {
            let state = store.read(id).unwrap();
            assert_eq!(state.status, "partial");
            assert_eq!(
                state.reason.as_deref(),
                Some(if kind == "missing" {
                    "history_missing"
                } else {
                    "history_corrupt"
                })
            );
            let observed = store.inspect_recovery(id, None).unwrap();
            assert_eq!(observed.history_corrupt, kind != "missing");
            assert!(observed.history_bytes.is_none());
        }
    }
}

fn cli(root: &Path, request: serde_json::Value) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .current_dir(root)
        .env("HOME", root.join("unused-home"))
        .arg("memory")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn cli_rejects_literal_only_history_update_without_a_saved_revision() {
    let (temp, vault, ctx) = setup();
    let store = Store::open(&vault).unwrap();
    let id = store.create(uid(1), create(&ctx)).unwrap().memory_id;
    let expected = baseline(&store, id);
    let current = fs::read(file(&vault, id, "Memoria.md")).unwrap();
    let history = fs::read(file(&vault, id, "Historial.md")).unwrap();
    let doc = MemoryDocument::parse(id, &expected.accepted_markdown).unwrap();
    let body = doc.body().replace(
        &history_link(id),
        &format!("```text\n{}\n```", history_link(id)),
    );
    let response = cli(
        temp.path(),
        serde_json::json!({"protocol_version":1,"request_id":uid(2),"operation":"update","vault":vault,"arguments":{"memory_id":id,"expected":expected,"body":body}}),
    );
    assert_eq!(response["result"]["status"], "invalid");
    assert_eq!(fs::read(file(&vault, id, "Memoria.md")).unwrap(), current);
    assert_eq!(fs::read(file(&vault, id, "Historial.md")).unwrap(), history);
    assert!(!file(&vault, id, ".pending.json").exists());
}

#[test]
fn cli_advertises_replacement_rejects_unknown_fields_and_exposes_corruption_bytes() {
    let (temp, vault, ctx) = setup();
    let old = uid(900);
    let before = bind_missing(&ctx, old);
    let req = |n, op: &str, args| serde_json::json!({"protocol_version":1,"request_id":uid(n),"operation":op,"vault":vault,"arguments":args});
    let caps = cli(temp.path(), req(1, "capabilities", serde_json::json!({})));
    assert!(
        caps["result"]["operations"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("replace_missing"))
    );
    let mut args = serde_json::to_value(replacement(&ctx, old, &before)).unwrap();
    args["force"] = serde_json::json!(true);
    assert_eq!(
        cli(temp.path(), req(2, "replace_missing", args))["result"]["status"],
        "invalid"
    );
    let saved = cli(
        temp.path(),
        req(
            3,
            "replace_missing",
            serde_json::to_value(replacement(&ctx, old, &before)).unwrap(),
        ),
    );
    assert_eq!(saved["result"]["status"], "saved");
    assert_eq!(saved["result"]["replaced_memory_id"], old.to_string());
    let id: Uuid = serde_json::from_value(saved["result"]["memory_id"].clone()).unwrap();
    fs::write(file(&vault, id, "Historial.md"), [0xff]).unwrap();
    let read = cli(
        temp.path(),
        req(4, "read", serde_json::json!({"memory_id":id})),
    );
    assert_eq!(read["result"]["status"], "partial");
    assert_eq!(read["result"]["reason"], "history_corrupt");
    let inspect = cli(
        temp.path(),
        req(5, "inspect_recovery", serde_json::json!({"memory_id":id})),
    );
    assert_eq!(inspect["result"]["status"], "partial");
    assert_eq!(inspect["result"]["reason"], "history_corrupt");
    assert_eq!(
        inspect["result"]["observation"]["history_bytes"],
        serde_json::json!([255])
    );
    assert_eq!(
        cli(
            temp.path(),
            req(
                6,
                "recover",
                serde_json::json!({"memory_id":id,"observation":inspect["result"]["observation"],"human_approved":true})
            )
        )["result"]["status"],
        "partial"
    );
}
