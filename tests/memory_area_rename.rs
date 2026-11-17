//! Area-name compatibility boundaries; exclusively synthetic temporary files.
use brain::memory::{
    CURRENT_SECTIONS, current_link,
    history::{AcceptedSnapshot, History},
    history_link,
    markdown::{ProjectAssociation, create_document},
    store::{Context, Create, Expected, Store},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn fixture() -> (tempfile::TempDir, PathBuf, Context) {
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
    (
        temp,
        vault,
        Context {
            working_directory: work.clone(),
            authorized_boundary: work,
        },
    )
}
fn create(context: &Context) -> Create {
    Create {
        context: context.clone(),
        title: "Area rename".into(),
        sections: std::array::from_fn(|_| "Evidence".into()),
        project: None,
        memory_subfolders: vec![],
    }
}
fn legacy(vault: &Path, context: &Context) -> (PathBuf, String, String) {
    let memory = id(100);
    let body = CURRENT_SECTIONS
        .iter()
        .map(|name| format!("## {name}\n\n"))
        .collect::<String>()
        + &history_link(memory);
    let old = create_document(memory, "Legacy accepted fixture", &body, None)
        .unwrap()
        .raw
        .replace("05-memorias/", "Memorias/");
    let mut history = History::new(memory).unwrap();
    history
        .push(AcceptedSnapshot {
            revision: 1,
            request_id: id(1),
            markdown: old.clone(),
        })
        .unwrap();
    // Encode the OLD snapshot's exact byte count, then change ONLY the header
    // to its legacy spelling, simulating a well-framed old accepted history.
    let raw = history.encode().unwrap().replacen(
        &current_link(memory),
        &format!("[[Memorias/{memory}/Memoria|Memoria]]"),
        1,
    );
    let dir = vault.join("Memorias").join(memory.to_string());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".lock"), "").unwrap();
    fs::write(dir.join("Memoria.md"), &old).unwrap();
    fs::write(dir.join("Historial.md"), &raw).unwrap();
    fs::write(
        context.working_directory.join(".brain.toml"),
        format!("binding_version=1\nmemory_id='{memory}'\n"),
    )
    .unwrap();
    (dir, old, raw)
}

#[test]
fn canonical_05_area_links_are_lazy_and_project_must_be_outside_it() {
    let (_temp, vault, ctx) = fixture();
    let store = Store::open(&vault).unwrap();
    assert!(!vault.join("05-memorias").exists());
    assert_eq!(store.read(id(100)).unwrap().status, "not_found");
    assert!(!vault.join("05-memorias").exists());
    assert_eq!(
        current_link(id(100)),
        format!("[[05-memorias/{}/Memoria|Memoria]]", id(100))
    );
    assert_eq!(
        history_link(id(100)),
        format!("[[05-memorias/{}/Historial|Historial]]", id(100))
    );
    let project = format!(
        "---\nbrain_type: project\nbrain_version: 1\nbrain_id: {}\ntitle: P\n---\n",
        id(200)
    );
    fs::create_dir(vault.join("05-memorias")).unwrap();
    fs::write(vault.join("05-memorias/P.md"), &project).unwrap();
    let mut args = create(&ctx);
    args.project = Some(ProjectAssociation {
        brain_project: id(200),
        path: "05-memorias/P.md".into(),
        label: "P".into(),
    });
    assert_eq!(store.create(id(1), args).unwrap_err().status, "invalid");
    assert!(!ctx.working_directory.join(".brain.toml").exists());
    fs::write(vault.join("01-proyectos/P.md"), &project).unwrap();
    let mut args = create(&ctx);
    args.project = Some(ProjectAssociation {
        brain_project: id(200),
        path: "01-proyectos/P.md".into(),
        label: "P".into(),
    });
    let saved = store.create(id(2), args).unwrap();
    let dir = vault.join("05-memorias").join(saved.memory_id.to_string());
    assert!(dir.join("Memoria.md").is_file());
    assert!(dir.join("Historial.md").is_file());
    assert!(!vault.join("Memorias").exists());
    assert_eq!(store.read(saved.memory_id).unwrap().status, "found");
}

#[test]
fn legacy_layout_is_not_an_alias_and_move_alone_cannot_migrate_history() {
    let (_temp, vault, ctx) = fixture();
    let (old_dir, current, history) = legacy(&vault, &ctx);
    let store = Store::open(&vault).unwrap();
    let marker = fs::read(ctx.working_directory.join(".brain.toml")).unwrap();
    assert_eq!(store.resolve(&ctx).unwrap()["status"], "not_found");
    assert_eq!(store.read(id(100)).unwrap().status, "not_found");
    assert!(!vault.join("05-memorias").exists());
    assert_eq!(
        fs::read_to_string(old_dir.join("Memoria.md")).unwrap(),
        current
    );
    assert_eq!(
        fs::read_to_string(old_dir.join("Historial.md")).unwrap(),
        history
    );
    // Simulate ONLY a directory move in this synthetic fixture, not an actual
    // migration command or permission to touch any user's stored Memory.
    fs::create_dir(vault.join("05-memorias")).unwrap();
    let moved = vault.join("05-memorias").join(id(100).to_string());
    fs::rename(old_dir, &moved).unwrap();
    let state = store.read(id(100)).unwrap();
    assert_eq!(state.status, "partial");
    assert_eq!(state.reason.as_deref(), Some("history_corrupt"));
    assert!(state.accepted_markdown.is_none());
    assert_eq!(
        store
            .update(
                id(100),
                id(3),
                Expected {
                    revision: 1,
                    accepted_markdown: current.clone()
                },
                "ignored"
            )
            .unwrap_err()
            .status,
        "partial"
    );
    let observed = store.inspect_recovery(id(100), Some(&ctx)).unwrap();
    assert!(observed.history_corrupt);
    assert_eq!(
        store
            .recover(id(100), Some(&ctx), observed, true)
            .unwrap_err()
            .status,
        "partial"
    );
    assert_eq!(
        fs::read_to_string(moved.join("Memoria.md")).unwrap(),
        current
    );
    assert_eq!(
        fs::read_to_string(moved.join("Historial.md")).unwrap(),
        history
    );
    assert_eq!(
        fs::read(ctx.working_directory.join(".brain.toml")).unwrap(),
        marker
    );
    assert!(!moved.join(".pending.json").exists());
}

#[test]
fn changing_only_history_header_still_preserves_rejects_old_snapshot_relations() {
    let (_temp, vault, ctx) = fixture();
    let (old_dir, current, history) = legacy(&vault, &ctx);
    fs::create_dir(vault.join("05-memorias")).unwrap();
    let moved = vault.join("05-memorias").join(id(100).to_string());
    fs::rename(old_dir, &moved).unwrap();
    let header_changed = history.replacen(
        &format!("[[Memorias/{}/Memoria|Memoria]]", id(100)),
        &current_link(id(100)),
        1,
    );
    fs::write(moved.join("Historial.md"), &header_changed).unwrap();
    let parsed = History::parse(id(100), &header_changed).unwrap();
    assert_eq!(parsed.latest().unwrap().markdown, current);
    let store = Store::open(&vault).unwrap();
    let state = store.read(id(100)).unwrap();
    assert_eq!(state.status, "partial");
    assert_eq!(state.reason.as_deref(), Some("history_document_invalid"));
    assert_eq!(
        store
            .update(
                id(100),
                id(3),
                Expected {
                    revision: 1,
                    accepted_markdown: current.clone()
                },
                "ignored"
            )
            .unwrap_err()
            .status,
        "partial"
    );
    assert_eq!(
        fs::read_to_string(moved.join("Memoria.md")).unwrap(),
        current
    );
    assert_eq!(
        fs::read_to_string(moved.join("Historial.md")).unwrap(),
        header_changed
    );
    assert!(!moved.join(".pending.json").exists());
}
