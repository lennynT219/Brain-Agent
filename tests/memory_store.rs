use brain::memory::{
    binding::{Binding, Resolution, resolve},
    history::History,
    history_link,
    markdown::{MemoryDocument, ProjectAssociation},
    store::{Context, Create, Expected, Fault, Store},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};
use uuid::Uuid;

fn request(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn vault(root: &Path) -> PathBuf {
    let vault = root.join("vault");
    for child in [
        "00-limbo",
        "01-proyectos",
        "02-areas",
        "03-recursos",
        "04-archivo",
        "99-sistema/plantillas",
        "99-sistema/documentacion",
    ] {
        fs::create_dir_all(vault.join(child)).unwrap();
    }
    vault
}
fn context(root: &Path) -> Context {
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    Context {
        working_directory: work.clone(),
        authorized_boundary: work,
    }
}
fn create(context: &Context) -> Create {
    Create {
        context: context.clone(),
        title: "Test context".into(),
        sections: [
            "Goal".into(),
            "Decisions".into(),
            "In progress".into(),
            "Synthetic evidence".into(),
            "Next".into(),
        ],
        project: None,
        memory_subfolders: vec![],
    }
}
fn path(vault: &Path, id: Uuid, name: &str) -> PathBuf {
    vault.join("05-memorias").join(id.to_string()).join(name)
}
fn expected(store: &Store, id: Uuid) -> Expected {
    let state = store.read(id).unwrap();
    assert_eq!(state.status, "found");
    Expected {
        revision: state.revision.unwrap(),
        accepted_markdown: state.accepted_markdown.unwrap(),
    }
}
fn changed_body(before: &Expected) -> String {
    MemoryDocument::parse(
        Uuid::parse_str(
            before
                .accepted_markdown
                .split("brain_id:")
                .nth(1)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .trim()
                .trim_matches('"'),
        )
        .unwrap(),
        &before.accepted_markdown,
    )
    .unwrap()
    .body()
    .replace("In progress", "Done")
}

#[test]
fn create_read_update_preserves_history_identity_and_exact_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let before = store.resolve(&ctx).unwrap();
    assert_eq!(before["status"], "unbound");
    assert!(!vault.join("05-memorias").exists());
    assert!(!ctx.working_directory.join(".brain.toml").exists());
    let saved = store.create(request(1), create(&ctx)).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        Binding::parse(&fs::read_to_string(ctx.working_directory.join(".brain.toml")).unwrap())
            .unwrap()
            .memory_id,
        saved.memory_id
    );
    let first = expected(&store, saved.memory_id);
    assert!(
        first
            .accepted_markdown
            .contains(&history_link(saved.memory_id))
    );
    let next = store
        .update(
            saved.memory_id,
            request(2),
            first.clone(),
            &changed_body(&first),
        )
        .unwrap();
    assert_eq!(next.revision, 2);
    let current = fs::read_to_string(path(&vault, saved.memory_id, "Memoria.md")).unwrap();
    let history = History::parse(
        saved.memory_id,
        &fs::read_to_string(path(&vault, saved.memory_id, "Historial.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(history.snapshots.len(), 2);
    assert_eq!(history.snapshots[0].markdown, first.accepted_markdown);
    assert_eq!(history.latest().unwrap().markdown, current);
    assert!(!path(&vault, saved.memory_id, ".pending.json").exists());
    assert!(!ctx.working_directory.join(".brain.pending.toml").exists());
    assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 1);
}

#[test]
fn stale_writers_and_busy_locks_never_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
    let old = expected(&store, id);
    let body = changed_body(&old);
    store.update(id, request(2), old.clone(), &body).unwrap();
    let bytes = fs::read(path(&vault, id, "Memoria.md")).unwrap();
    assert_eq!(
        store.update(id, request(3), old, &body).unwrap_err().status,
        "conflict"
    );
    assert_eq!(fs::read(path(&vault, id, "Memoria.md")).unwrap(), bytes);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path(&vault, id, ".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(store.read(id).unwrap_err().status, "busy");
    drop(lock);
    assert_eq!(store.read(id).unwrap().status, "found");
}

#[test]
fn manual_review_is_exact_preserves_unknown_yaml_crlf_and_fences() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
    let baseline = expected(&store, id);
    let manual = baseline
        .accepted_markdown
        .replacen("title:", "unknown: {custom: [1, 2]} # keep me\ntitle:", 1)
        .replace(
            "In progress",
            "Human edit\n\n````markdown\n## Not a section\n```\n````",
        )
        .replace('\n', "\r\n")
        .trim_end_matches(['\r', '\n'])
        .to_string();
    fs::write(path(&vault, id, "Memoria.md"), &manual).unwrap();
    let state = store.read(id).unwrap();
    assert_eq!(state.status, "manual_review");
    assert_eq!(state.observed_markdown.as_deref(), Some(manual.as_str()));
    assert_eq!(
        store
            .update(id, request(2), baseline.clone(), "ignored")
            .unwrap_err()
            .status,
        "manual_review"
    );
    let approved = state.approval_candidate.unwrap();
    assert!(approved.contains("unknown: {custom: [1, 2]} # keep me\r\n"));
    assert!(!approved.ends_with('\n'));
    assert_eq!(
        store
            .accept_external(id, request(2), baseline.clone(), &manual, &approved, false)
            .unwrap_err()
            .status,
        "manual_review"
    );
    fs::write(path(&vault, id, "Memoria.md"), format!("{manual} ")).unwrap();
    assert_eq!(
        store
            .accept_external(id, request(2), baseline.clone(), &manual, &approved, true)
            .unwrap_err()
            .status,
        "conflict"
    );
    fs::write(path(&vault, id, "Memoria.md"), &manual).unwrap();
    store
        .accept_external(id, request(2), baseline, &manual, &approved, true)
        .unwrap();
    assert_eq!(
        fs::read_to_string(path(&vault, id, "Memoria.md")).unwrap(),
        approved
    );
    assert_eq!(
        History::parse(
            id,
            &fs::read_to_string(path(&vault, id, "Historial.md")).unwrap()
        )
        .unwrap()
        .latest()
        .unwrap()
        .markdown,
        approved
    );
    let accepted = expected(&store, id);
    let body = MemoryDocument::parse(id, &accepted.accepted_markdown)
        .unwrap()
        .body()
        .replace("Human edit", "Agent successor");
    store.update(id, request(3), accepted, &body).unwrap();
    assert!(
        fs::read_to_string(path(&vault, id, "Memoria.md"))
            .unwrap()
            .contains("unknown: {custom: [1, 2]} # keep me\r\n")
    );
}

#[test]
fn missing_corrupt_and_wrong_identity_history_are_partial_not_adopted() {
    for corrupt in [
        None,
        Some("truncated"),
        Some("# Historial\n\n<!-- brain-history-v1 wrong -->"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let vault = vault(temp.path());
        let ctx = context(temp.path());
        let store = Store::open(&vault).unwrap();
        let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
        let baseline = expected(&store, id);
        let raw = fs::read(path(&vault, id, "Memoria.md")).unwrap();
        match corrupt {
            None => fs::remove_file(path(&vault, id, "Historial.md")).unwrap(),
            Some(value) => fs::write(path(&vault, id, "Historial.md"), value).unwrap(),
        };
        assert_eq!(store.read(id).unwrap().status, "partial");
        assert_eq!(
            store
                .update(id, request(2), baseline, "ignored")
                .unwrap_err()
                .status,
            "partial"
        );
        assert_eq!(fs::read(path(&vault, id, "Memoria.md")).unwrap(), raw);
    }
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let store = Store::open(&vault).unwrap();
    assert_eq!(store.read(request(999)).unwrap().status, "not_found");
    assert_eq!(
        Store::open(&temp.path().join("missing-vault"))
            .err()
            .unwrap()
            .status,
        "unavailable"
    );
}

#[test]
fn declared_children_lazy_creation_and_copies_share_memory() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let child = ctx.working_directory.join("child");
    let nested = child.join("nested");
    let other = ctx.working_directory.join("other");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(&other).unwrap();
    let mut args = create(&ctx);
    args.memory_subfolders = vec!["child".into(), "child/nested".into(), "other".into()];
    let parent = store.create(request(1), args).unwrap().memory_id;
    assert!(!child.join(".brain.toml").exists());
    assert!(!nested.join(".brain.toml").exists());
    assert!(!other.join(".brain.toml").exists());
    let child_ctx = Context {
        working_directory: nested.clone(),
        authorized_boundary: ctx.authorized_boundary.clone(),
    };
    let child_id = store
        .create(request(2), create(&child_ctx))
        .unwrap()
        .memory_id;
    assert_ne!(parent, child_id);
    assert!(nested.join(".brain.toml").exists());
    assert!(!child.join(".brain.toml").exists());
    assert!(!other.join(".brain.toml").exists());
    let copied = ctx.working_directory.join("copy");
    fs::create_dir(&copied).unwrap();
    fs::copy(nested.join(".brain.toml"), copied.join(".brain.toml")).unwrap();
    assert!(
        matches!(resolve(&copied,&ctx.authorized_boundary).unwrap(),Resolution::Bound{binding,..}if binding.memory_id==child_id)
    );
    assert_eq!(store.read(child_id).unwrap().status, "found");
    assert_eq!(
        store
            .create(request(3), create(&child_ctx))
            .unwrap_err()
            .status,
        "conflict"
    );
    assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 2);
}

#[test]
fn concurrent_creators_publish_one_identity() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let barrier = Arc::new(Barrier::new(2));
    let handles = (1..=2)
        .map(|n| {
            let vault = vault.clone();
            let ctx = ctx.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&vault).unwrap();
                barrier.wait();
                store.create(request(n), create(&ctx))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|f| matches!(f.status.as_str(), "busy" | "conflict"))
    );
    assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 1);
}

#[test]
fn interrupted_updates_need_exact_approved_recovery_not_read_side_repair() {
    for fault in [
        Fault::AfterPending,
        Fault::AfterCurrent,
        Fault::AfterHistory,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let vault = vault(temp.path());
        let ctx = context(temp.path());
        let store = Store::open(&vault).unwrap();
        let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
        let baseline = expected(&store, id);
        let faulty = Store::open(&vault).unwrap().with_fault(fault);
        assert_eq!(
            faulty
                .update(id, request(2), baseline.clone(), &changed_body(&baseline))
                .unwrap_err()
                .status,
            "partial"
        );
        let observed = store.inspect_recovery(id, None).unwrap();
        assert!(observed.pending.is_some());
        assert_eq!(store.read(id).unwrap().status, "partial");
        assert_eq!(store.inspect_recovery(id, None).unwrap(), observed);
        assert_eq!(
            store
                .recover(id, None, observed.clone(), false)
                .unwrap_err()
                .status,
            "manual_review"
        );
        let mut stale = observed.clone();
        stale.current = Some("different".into());
        assert_eq!(
            store.recover(id, None, stale, true).unwrap_err().status,
            "conflict"
        );
        let saved = store.recover(id, None, observed, true).unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(store.read(id).unwrap().status, "found");
        assert_eq!(
            History::parse(
                id,
                &fs::read_to_string(path(&vault, id, "Historial.md")).unwrap()
            )
            .unwrap()
            .snapshots
            .len(),
            2
        );
    }
}

#[test]
fn interrupted_creation_keeps_identity_and_recovery_materializes_marker() {
    for fault in [
        Fault::AfterPending,
        Fault::AfterCurrent,
        Fault::AfterHistory,
        Fault::AfterMarker,
        Fault::AfterReservationCleanup,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let vault = vault(temp.path());
        let ctx = context(temp.path());
        let faulty = Store::open(&vault).unwrap().with_fault(fault);
        let failure = faulty.create(request(1), create(&ctx)).unwrap_err();
        assert_eq!(failure.status, "partial");
        let id = failure.memory_id.unwrap();
        let store = Store::open(&vault).unwrap();
        if fault == Fault::AfterReservationCleanup {
            assert_eq!(store.read(id).unwrap().status, "partial");
            assert_eq!(
                store.create(request(2), create(&ctx)).unwrap_err().status,
                "conflict"
            );
        } else {
            assert_eq!(store.resolve(&ctx).unwrap()["status"], "partial");
            assert_eq!(
                store.create(request(2), create(&ctx)).unwrap_err().status,
                "partial"
            );
        }
        let observation = store.inspect_recovery(id, Some(&ctx)).unwrap();
        store.recover(id, Some(&ctx), observation, true).unwrap();
        assert_eq!(store.read(id).unwrap().status, "found");
        assert!(ctx.working_directory.join(".brain.toml").exists());
        assert!(!ctx.working_directory.join(".brain.pending.toml").exists());
        assert_eq!(fs::read_dir(vault.join("05-memorias")).unwrap().count(), 1);
    }
}

#[test]
fn pre_intent_interruption_preserves_reservation_without_guessing_content() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let failure = Store::open(&vault)
        .unwrap()
        .with_fault(Fault::AfterReservation)
        .create(request(1), create(&ctx))
        .unwrap_err();
    let id = failure.memory_id.unwrap();
    let store = Store::open(&vault).unwrap();
    assert_eq!(store.resolve(&ctx).unwrap()["status"], "partial");
    assert_eq!(
        store.create(request(2), create(&ctx)).unwrap_err().status,
        "partial"
    );
    assert!(!vault.join("05-memorias").exists());
    let observation = store.inspect_recovery(id, Some(&ctx)).unwrap();
    assert!(observation.pending.is_none());
    assert_eq!(
        store
            .recover(id, Some(&ctx), observation, true)
            .unwrap_err()
            .status,
        "not_found"
    );
    assert!(ctx.working_directory.join(".brain.pending.toml").exists());
}

#[test]
fn optional_project_is_explicit_real_and_outside_memorias() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let project_id = request(900);
    fs::write(vault.join("01-proyectos/StelaShell.md"),format!("---\nbrain_type: project\nbrain_version: 1\nbrain_id: {project_id}\ntitle: StelaShell\n---\n")).unwrap();
    let store = Store::open(&vault).unwrap();
    let mut args = create(&ctx);
    args.project = Some(ProjectAssociation {
        brain_project: project_id,
        path: "01-proyectos/StelaShell.md".into(),
        label: "StelaShell".into(),
    });
    let id = store.create(request(1), args).unwrap().memory_id;
    assert!(
        expected(&store, id)
            .accepted_markdown
            .contains("[[01-proyectos/StelaShell|StelaShell]]")
    );
    assert!(!path(&vault, id, "StelaShell.md").exists());
    let another = temp.path().join("another");
    fs::create_dir(&another).unwrap();
    let mut args = create(&Context {
        working_directory: another.clone(),
        authorized_boundary: another.clone(),
    });
    args.project = Some(ProjectAssociation {
        brain_project: request(901),
        path: "01-proyectos/StelaShell.md".into(),
        label: "StelaShell".into(),
    });
    assert_eq!(
        store.create(request(2), args).unwrap_err().status,
        "invalid"
    );
    assert!(!another.join(".brain.pending.toml").exists());
}

#[test]
fn recovery_rejects_structurally_invalid_retained_documents() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
    let baseline = expected(&store, id);
    let faulty = Store::open(&vault).unwrap().with_fault(Fault::AfterPending);
    assert!(
        faulty
            .update(id, request(2), baseline.clone(), &changed_body(&baseline))
            .is_err()
    );
    let pending_path = path(&vault, id, ".pending.json");
    let mut pending: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&pending_path).unwrap()).unwrap();
    let mut before = History::parse(id, pending["before_history"].as_str().unwrap()).unwrap();
    before.snapshots[0].markdown = before.snapshots[0]
        .markdown
        .replace("## Objective and scope", "## Invalid section");
    let invalid_before = before.encode().unwrap();
    let mut next = History::parse(id, pending["next_history"].as_str().unwrap()).unwrap();
    next.snapshots[0] = before.snapshots[0].clone();
    pending["before_history"] = serde_json::json!(invalid_before);
    pending["next_history"] = serde_json::json!(next.encode().unwrap());
    fs::write(path(&vault, id, "Historial.md"), &invalid_before).unwrap();
    fs::write(&pending_path, serde_json::to_string(&pending).unwrap()).unwrap();
    let observation = store.inspect_recovery(id, None).unwrap();
    assert!(store.recover(id, None, observation.clone(), true).is_err());
    assert_eq!(store.inspect_recovery(id, None).unwrap(), observation);
    assert_eq!(store.read(id).unwrap().status, "partial");
}

#[test]
fn unchanged_context_does_not_append_a_revision() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let id = store.create(request(1), create(&ctx)).unwrap().memory_id;
    let baseline = expected(&store, id);
    let doc = MemoryDocument::parse(id, &baseline.accepted_markdown).unwrap();
    let history = fs::read(path(&vault, id, "Historial.md")).unwrap();
    let saved = store.update(id, request(2), baseline, doc.body()).unwrap();
    assert_eq!(saved.status, "unchanged");
    assert_eq!(saved.revision, 1);
    assert_eq!(fs::read(path(&vault, id, "Historial.md")).unwrap(), history);
}

#[test]
fn change_log_records_only_changes_and_keeps_untouched_sections_exact() {
    let temp = tempfile::tempdir().unwrap();
    let vault = vault(temp.path());
    let ctx = context(temp.path());
    let store = Store::open(&vault).unwrap();
    let id = store
        .create_log(request(40), create(&ctx), "Se inició el proyecto.")
        .unwrap()
        .memory_id;
    let initial = expected(&store, id);
    let history = path(&vault, id, "Historial.md");
    let first = fs::read_to_string(&history).unwrap();
    let changes = [("Outcome status".into(), "Done\n\n".into())].into();
    let saved = store
        .update_sections(
            id,
            request(41),
            initial.clone(),
            &changes,
            "Se cerró el bloqueo porque llegó la aprobación.",
        )
        .unwrap();
    assert_eq!(saved.revision, 2);
    let next = expected(&store, id);
    assert_eq!(
        next.accepted_markdown,
        initial
            .accepted_markdown
            .replace("memory_revision: 1", "memory_revision: 2")
            .replace("In progress", "Done")
    );
    let second = fs::read_to_string(&history).unwrap();
    let entry = second.strip_prefix(&first).unwrap();
    assert!(entry.contains("Se cerró el bloqueo porque llegó la aprobación."));
    assert_eq!(entry.matches("\n- ").count(), 1);
    assert!(!second.contains("## Revision") && !second.contains("Synthetic evidence"));
    assert_eq!(
        store
            .update_sections(id, request(42), next.clone(), &changes, "Sin cambios.")
            .unwrap()
            .status,
        "unchanged"
    );
    assert_eq!(
        store
            .update_sections(id, request(43), initial, &changes, "Base antigua.")
            .unwrap_err()
            .status,
        "conflict"
    );
    let injected = [(
        "Outcome status".into(),
        "## Relevant evidence\nDuplicado\n".into(),
    )]
    .into();
    assert_eq!(
        store
            .update_sections(id, request(44), next, &injected, "Inválido.")
            .unwrap_err()
            .status,
        "invalid"
    );
    assert_eq!(fs::read_to_string(history).unwrap(), second);
}

#[test]
fn change_log_recovers_exact_intents_and_manual_edits_still_require_approval() {
    for fault in [
        Fault::AfterPending,
        Fault::AfterCurrent,
        Fault::AfterHistory,
        Fault::AfterHead,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let vault = vault(temp.path());
        let ctx = context(temp.path());
        let store = Store::open(&vault).unwrap();
        let id = store
            .create_log(request(60), create(&ctx), "Inicio.")
            .unwrap()
            .memory_id;
        let initial = expected(&store, id);
        let current = path(&vault, id, "Memoria.md");
        let draft = initial
            .accepted_markdown
            .replace("In progress", "Un cambio humano");
        fs::write(&current, &draft).unwrap();
        let review = store.read(id).unwrap();
        assert_eq!(review.status, "manual_review");
        assert_eq!(
            review.accepted_markdown,
            Some(initial.accepted_markdown.clone())
        );
        let candidate = review.approval_candidate.unwrap();
        assert_eq!(
            store
                .accept_external(id, request(61), initial.clone(), &draft, &candidate, false)
                .unwrap_err()
                .status,
            "manual_review"
        );
        let fail_after = Store::open(&vault).unwrap().with_fault(fault);
        assert_eq!(
            fail_after
                .accept_external(id, request(62), initial, &draft, &candidate, true)
                .unwrap_err()
                .status,
            "partial"
        );
        assert_eq!(store.read(id).unwrap().status, "partial");
        let observation = store.inspect_recovery(id, None).unwrap();
        let mut stale = observation.clone();
        stale.accepted_head = None;
        assert_eq!(
            store.recover(id, None, stale, true).unwrap_err().status,
            "conflict"
        );
        assert_eq!(
            store.recover(id, None, observation, true).unwrap().revision,
            2
        );
        assert_eq!(expected(&store, id).accepted_markdown, candidate);
        let history = fs::read_to_string(path(&vault, id, "Historial.md")).unwrap();
        assert!(history.contains("Outcome status"));
        assert!(!history.contains("Un cambio humano"));
        assert_eq!(history.matches("\n- ").count(), 2);
    }
}

#[test]
fn project_only_v2_first_use_recovers_without_replacing_identity_or_principal() {
    for log in [false, true] {
        for fault in [Fault::AfterPending, Fault::AfterCurrent, Fault::AfterHistory,
            Fault::AfterHead, Fault::BeforeMarker, Fault::AfterMarker, Fault::AfterReservationCleanup] {
            if !log && fault == Fault::AfterHead { continue; }
            let temp = tempfile::tempdir().unwrap();
            let vault = vault(temp.path());
            let ctx = context(temp.path());
            let project_id = request(300);
            let project = vault.join("01-proyectos/Principal");
            fs::create_dir_all(project.join("notas")).unwrap();
            fs::write(project.join("Proyecto.md"), format!("---\nbrain_type: project\nbrain_version: 1\nbrain_id: {project_id}\ntitle: Principal\n---\nProyecto\n")).unwrap();
            let marker = format!("# conservar\nbinding_version=2\nmemory_subfolders=['child']\n[principal_project]\nproject_id='{project_id}'\nproject_path='01-proyectos/Principal/Proyecto.md'\nallow_write=true\n");
            let marker_path = ctx.working_directory.join(".brain.toml");
            fs::write(&marker_path, &marker).unwrap();
            let store = Store::open(&vault).unwrap();
            assert_eq!(store.resolve(&ctx).unwrap()["status"], "unbound");
            let faulty = Store::open(&vault).unwrap().with_fault(fault);
            let result = if log { faulty.create_log(request(301), create(&ctx), "Inicio útil.") }
                else { faulty.create(request(301), create(&ctx)) };
            let error = result.unwrap_err();
            assert_eq!(error.status, "partial");
            let id = error.memory_id.unwrap();
            assert_eq!(store.resolve(&ctx).unwrap()["status"], "partial");
            let observation = store.inspect_recovery(id, Some(&ctx)).unwrap();
            store.recover(id, Some(&ctx), observation, true).unwrap();
            let current = expected(&store, id);
            assert!(current.accepted_markdown.contains(&project_id.to_string()));
            let published = fs::read_to_string(marker_path).unwrap();
            assert!(published.ends_with(&marker));
            assert_eq!(Binding::parse(&published).unwrap().memory_id, id);
            assert!(!path(&vault, id, ".reassociation.json").exists());
            assert_eq!(store.resolve(&ctx).unwrap()["binding"]["binding_version"], 1);
        }
    }
}
