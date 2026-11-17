//! Isolated fixtures only: no personal vault/config/marker is accessed.
use brain::{
    document::DocumentMetadata,
    memory::{
        binding::{Binding, Resolution, resolve},
        current_link,
        history::{AcceptedSnapshot, History},
        history_link,
    },
};
use std::fs;
use uuid::Uuid;

const ID: &str = "550e8400-e29b-41d4-a716-446655440000";
fn id() -> Uuid {
    Uuid::parse_str(ID).unwrap()
}
fn metadata(extra: &str) -> String {
    format!(
        "brain_type = 'memory'\nbrain_version = 1\nbrain_id = '{ID}'\ntitle = 'Context'\nmemory_revision = 1\n{extra}"
    )
}
fn marker(extra: &str) -> String {
    format!("binding_version = 1\nmemory_id = '{ID}'\n{extra}")
}

#[test]
fn memory_metadata_is_additive_and_project_optional() {
    for extra in [
        "",
        "brain_project = '550e8400-e29b-41d4-a716-446655440001'\nproject_link = '[[01-proyectos/StelaShell|StelaShell]]'\n",
    ] {
        // TOML is a Serde test harness only, not the Markdown storage format.
        let value: DocumentMetadata = toml::from_str(&metadata(extra)).unwrap();
        assert!(value.validate().is_ok());
        assert_eq!(value.common().brain_id, id());
        let encoded = toml::to_string(&value).unwrap();
        assert_eq!(toml::from_str::<DocumentMetadata>(&encoded).unwrap(), value);
        if extra.is_empty() {
            assert!(!encoded.contains("project"));
        }
    }
    for extra in [
        "brain_project = '550e8400-e29b-41d4-a716-446655440001'",
        "project_link = '[[01-proyectos/StelaShell|StelaShell]]'",
        "brain_project = '550e8400-e29b-41d4-a716-446655440001'\nproject_link = '[[StelaShell]]'",
        "brain_project = '550e8400-e29b-41d4-a716-446655440001'\nproject_link = '[[../StelaShell|Project]]'",
    ] {
        let value: DocumentMetadata = toml::from_str(&metadata(extra)).unwrap();
        assert!(value.validate().is_err());
    }
    for input in [
        metadata("").replace("memory_revision = 1", "memory_revision = 0"),
        metadata("").replace(ID, &Uuid::nil().to_string()),
    ] {
        let value: DocumentMetadata = toml::from_str(&input).unwrap();
        assert!(value.validate().is_err());
    }
    assert!(
        toml::from_str::<DocumentMetadata>(&metadata("").replace("memory_revision = 1", ""))
            .is_err()
    );
    assert!(toml::from_str::<DocumentMetadata>(&metadata("brain_project = 1")).is_err());
}

#[test]
fn marker_has_only_binding_fields_and_explicit_portable_paths() {
    let parsed = Binding::parse(&marker("memory_subfolders = ['child', 'nested/child']")).unwrap();
    assert_eq!(parsed.memory_subfolders.len(), 2);
    for extra in [
        "vault_path = '/somewhere'",
        "brain_project = 'anything'",
        "permissions = 'all'",
        "memory_subfolders = ['../escape']",
        "memory_subfolders = ['/absolute']",
        "memory_subfolders = ['child//nested']",
        "memory_subfolders = ['child/./nested']",
        "memory_subfolders = ['a', 'a']",
        "memory_subfolders = ['a*']",
        "memory_subfolders = ['a\\b']",
    ] {
        assert!(Binding::parse(&marker(extra)).is_err(), "{extra}");
    }
    assert!(
        Binding::parse(&marker("").replace("binding_version = 1", "binding_version = 2")).is_err()
    );
    assert!(Binding::parse(&marker("").replace(ID, &Uuid::nil().to_string())).is_err());
}

#[test]
fn resolution_is_read_only_lazy_and_nearest_marker_wins() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let child = root.join("child");
    let nested = child.join("nested");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(resolve(&nested, root).unwrap(), Resolution::Unbound);
    assert!(!root.join(".brain.toml").exists());
    fs::write(
        root.join(".brain.toml"),
        marker("memory_subfolders = ['child']"),
    )
    .unwrap();
    assert_eq!(
        resolve(&nested, root).unwrap(),
        Resolution::Declared {
            directory: child.clone(),
            declared_by: root.join(".brain.toml")
        }
    );
    assert!(!child.join(".brain.toml").exists());
    assert!(!root.join("05-memorias").exists());
    fs::write(child.join(".brain.toml"), marker("")).unwrap();
    assert!(
        matches!(resolve(&nested, root).unwrap(), Resolution::Bound { directory, binding } if directory == child && binding.memory_id == id())
    );
    // Without a declaration, ordinary subdirectories inherit enclosing context.
    fs::write(root.join(".brain.toml"), marker("")).unwrap();
    fs::remove_file(child.join(".brain.toml")).unwrap();
    assert!(
        matches!(resolve(&nested, root).unwrap(), Resolution::Bound { directory, .. } if directory == root)
    );
    let outside = tempfile::tempdir().unwrap();
    assert!(resolve(outside.path(), root).is_err());
}

#[test]
fn moves_and_copies_keep_the_same_identity_without_vault_scanning() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    fs::create_dir(&original).unwrap();
    fs::write(original.join(".brain.toml"), marker("")).unwrap();
    let moved = temp.path().join("moved");
    fs::rename(&original, &moved).unwrap();
    let copied = temp.path().join("copy");
    fs::create_dir(&copied).unwrap();
    fs::copy(moved.join(".brain.toml"), copied.join(".brain.toml")).unwrap();
    for path in [&moved, &copied] {
        assert!(
            matches!(resolve(path, temp.path()).unwrap(), Resolution::Bound { binding, .. } if binding.memory_id == id())
        );
    }
}

#[test]
fn marker_io_errors_are_not_unbound() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".brain.toml")).unwrap();
    assert!(resolve(temp.path(), temp.path()).is_err());
    let missing = temp.path().join("missing");
    let error = resolve(&missing, temp.path()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn symlink_markers_and_below_boundary_traversal_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("real-marker"), marker("")).unwrap();
    symlink(
        temp.path().join("real-marker"),
        temp.path().join(".brain.toml"),
    )
    .unwrap();
    assert!(resolve(temp.path(), temp.path()).is_err());
    fs::remove_file(temp.path().join(".brain.toml")).unwrap();
    fs::create_dir(temp.path().join("real")).unwrap();
    symlink(temp.path().join("real"), temp.path().join("alias")).unwrap();
    assert!(resolve(&temp.path().join("alias"), temp.path()).is_err());
}

#[test]
fn full_snapshot_frames_preserve_exact_bytes_and_safe_fences() {
    let mut history = History {
        memory_id: id(),
        snapshots: vec![],
    };
    let snapshots = [
        "---\r\nunknown: preserved # comment\r\n---\r\n\r\n```markdown\r\nold\r\n```\r\n",
        "unicode á🦀\n````````\n<!-- end-snapshot -->\nno final newline",
        "",
    ];
    for (i, content) in snapshots.iter().enumerate() {
        history.snapshots.push(AcceptedSnapshot {
            revision: (i + 1) as u64,
            request_id: Uuid::from_u128((i + 1) as u128),
            markdown: content.to_string(),
        });
    }
    let encoded = history.encode().unwrap();
    assert!(encoded.contains(&current_link(id())));
    assert_eq!(History::parse(id(), &encoded).unwrap(), history);
    assert_eq!(
        history_link(id()),
        format!("[[05-memorias/{ID}/Historial|Historial]]")
    );
    // Every proper nonempty-frame truncation fails rather than adopting a tail.
    let last_frame = encoded.rfind("## Revision ").unwrap();
    for (cut, _) in encoded.char_indices().filter(|(i, _)| *i > last_frame) {
        assert!(History::parse(id(), &encoded[..cut]).is_err(), "cut={cut}");
    }
}

#[test]
fn corrupt_frames_identity_revisions_and_duplicate_requests_fail() {
    let history = History {
        memory_id: id(),
        snapshots: vec![AcceptedSnapshot {
            revision: 1,
            request_id: Uuid::from_u128(1),
            markdown: "saved".into(),
        }],
    };
    let encoded = history.encode().unwrap();
    assert!(History::parse(Uuid::from_u128(2), &encoded).is_err());
    for changed in [
        encoded.replace("## Revision 1", "## Revision 2"),
        encoded.replace(" 5 3 -->", " 99999999999999999999 3 -->"),
        format!("{encoded}garbage"),
        encoded.replace("end-snapshot", "bad-footer"),
    ] {
        assert!(History::parse(id(), &changed).is_err());
    }
    let mut reused = history.clone();
    reused.snapshots.push(AcceptedSnapshot {
        revision: 2,
        ..history.snapshots[0].clone()
    });
    assert!(reused.encode().is_err());
    let mut skipped = history;
    skipped.snapshots[0].revision = 0;
    assert!(skipped.encode().is_err());
    let successor = AcceptedSnapshot {
        revision: 1,
        request_id: Uuid::from_u128(3),
        markdown: "candidate".into(),
    };
    let mut invalid_identity = History {
        memory_id: Uuid::nil(),
        snapshots: vec![],
    };
    assert!(invalid_identity.push(successor.clone()).is_err());
    assert!(
        skipped
            .push(AcceptedSnapshot {
                revision: 1,
                ..successor
            })
            .is_err()
    );
    assert!(History::new(id()).unwrap().encode().is_err());
}

#[test]
fn snapshot_lengths_are_utf8_bytes_not_characters() {
    let mut history = History::new(id()).unwrap();
    history
        .push(AcceptedSnapshot {
            revision: 1,
            request_id: Uuid::from_u128(1),
            markdown: "á🦀".into(), // U+00E1 = 2 bytes, U+1F980 = 4 bytes.
        })
        .unwrap();
    let encoded = history.encode().unwrap();
    assert!(encoded.contains("00000000-0000-0000-0000-000000000001 6 3 -->"));
    assert!(History::parse(id(), &encoded.replace(" 6 3 -->", " 3 3 -->")).is_err());
    assert_eq!(
        History::parse(id(), &encoded)
            .unwrap()
            .latest()
            .unwrap()
            .markdown,
        "á🦀"
    );
}
