use brain::{
    document::{CommonMetadata, DOCUMENT_VERSION, DocumentMetadata},
    memory::*,
};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use uuid::Uuid;

const PROJECT: Uuid = Uuid::from_u128(1);
const MEMORY: Uuid = Uuid::from_u128(2);
const ORIGINAL: &[u8] =
    b"\xef\xbb\xbf---\r\n# comment\r\nunknown: [uno, dos]\r\n---\r\n# Human title\r\nBody\r\n";

fn project() -> DocumentMetadata {
    DocumentMetadata::Project(CommonMetadata {
        brain_version: DOCUMENT_VERSION,
        brain_id: PROJECT,
        title: "Synthetic project".into(),
    })
}
fn store(root: &Path) -> MemoryStore {
    MemoryStore::new(Binding::new(root.into(), &project(), PROJECT).unwrap())
}
fn metadata(session: &str) -> Metadata {
    Metadata {
        title: "Synthetic memory".into(),
        claim_kind: ClaimKind::AgentInference,
        session: session.into(),
        observed_at_unix_ms: 42,
        sources: vec![Source {
            reference: "fixture:source".into(),
            locator: None,
            revision: None,
        }],
    }
}
struct Lab {
    _temp: tempfile::TempDir,
    root: PathBuf,
    memory: PathBuf,
    store: MemoryStore,
}
impl Lab {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let memory = root
            .join(PROJECT.to_string())
            .join(COLLECTION)
            .join(MEMORY.to_string());
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("view.md"), ORIGINAL).unwrap();
        Self {
            store: store(&root),
            _temp: temp,
            root,
            memory,
        }
    }
    fn create(&self) {
        let result = self
            .store
            .create_from_view(MEMORY, metadata("session-a"))
            .unwrap();
        assert_eq!(result.revision, 1);
        assert_eq!(result.view, ViewState::Base);
        assert!(!result.needs_review);
    }
    fn revision(&self, n: u64) -> PathBuf {
        self.memory.join("revisions").join(format!("{n}.bmr"))
    }
    fn seed(&self, filename: u64, mut header: Header, body: &[u8]) {
        header.payload_len = body.len() as u64;
        let text = toml::to_string(&header).unwrap();
        let mut bytes = b"BRAINMR1".to_vec();
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(body);
        fs::write(self.revision(filename), bytes).unwrap();
    }
    fn header(&self) -> Header {
        self.store.read_revision(MEMORY, 1, 0).unwrap().header
    }
    fn files(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(self.memory.join("revisions"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        paths
    }
}

#[test]
fn raw_lifecycle_history_own_base_and_duplicate_create() {
    let lab = Lab::new();
    lab.create();
    let original_record = fs::read(lab.revision(1)).unwrap();
    let read = lab.store.read_current(MEMORY, MAX_READ_BYTES).unwrap();
    assert_eq!(read.bytes, ORIGINAL);
    assert_eq!(read.state, RevisionState::Current);
    assert_eq!(read.header.project_id, PROJECT);
    assert_eq!(read.header.collection, COLLECTION);
    assert_eq!(read.header.memory_id, MEMORY);
    assert_eq!(read.header.store_version, STORE_VERSION);
    let published = lab
        .store
        .publish_successor(MEMORY, 1, b"new inference", metadata("session-b"))
        .unwrap();
    assert_eq!(published.revision, 2);
    assert_eq!(published.view, ViewState::Stale);
    assert!(!published.needs_review);
    let current = lab.store.read_current(MEMORY, MAX_READ_BYTES).unwrap();
    assert_eq!(current.bytes, b"new inference");
    assert_eq!(current.header.parent, Some(1));
    assert_eq!(current.header.metadata.session, "session-b");
    assert_eq!(current.view, ViewState::Stale); // NOT a comparison against generated content.
    let old = lab.store.read_revision(MEMORY, 1, 3).unwrap();
    assert_eq!(old.bytes, &ORIGINAL[..3]);
    assert_eq!(old.total_bytes, ORIGINAL.len());
    assert!(old.truncated);
    assert_eq!(old.state, RevisionState::Historical);
    assert_eq!(fs::read(lab.revision(1)).unwrap(), original_record);
    assert_eq!(fs::read(lab.memory.join("view.md")).unwrap(), ORIGINAL);
    assert!(matches!(
        lab.store.create_from_view(MEMORY, metadata("retry")),
        Err(Error::AlreadyExists)
    ));
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, b"stale", metadata("a")),
        Err(Error::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(lab.files().len(), 2);
    assert!(matches!(
        lab.store.read_current(MEMORY, MAX_READ_BYTES + 1),
        Err(Error::Limit(_))
    ));
}

#[test]
fn manual_edits_require_review_but_history_is_accessible() {
    let lab = Lab::new();
    lab.create();
    lab.store
        .publish_successor(MEMORY, 1, b"generated", metadata("a"))
        .unwrap();
    let before: Vec<_> = lab.files().iter().map(|p| fs::read(p).unwrap()).collect();
    let manual = b"---\nunknown: changed-by-human\n---\nManual body";
    fs::write(lab.memory.join("view.md"), manual).unwrap();
    assert!(matches!(
        lab.store.read_current(MEMORY, 10),
        Err(Error::NeedsReview {
            view: ViewState::Diverged,
            ..
        })
    ));
    assert!(matches!(
        lab.store.publish_successor(MEMORY, 2, b"no", metadata("a")),
        Err(Error::NeedsReview { .. })
    ));
    let old = lab.store.read_revision(MEMORY, 1, MAX_READ_BYTES).unwrap();
    assert_eq!(old.bytes, ORIGINAL);
    assert_eq!(old.state, RevisionState::Historical);
    assert!(old.needs_review);
    assert_eq!(
        lab.store.read_revision(MEMORY, 2, 10).unwrap().state,
        RevisionState::Unresolved
    );
    assert_eq!(fs::read(lab.memory.join("view.md")).unwrap(), manual);
    assert_eq!(
        lab.files()
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn competing_heads_are_preserved_not_selected_by_number() {
    let lab = Lab::new();
    lab.create();
    lab.store
        .publish_successor(MEMORY, 1, b"branch a", metadata("a"))
        .unwrap();
    let mut header = lab.header();
    header.revision = 3;
    header.parent = Some(1);
    lab.seed(3, header, b"branch b");
    assert!(
        matches!(lab.store.read_current(MEMORY, 10), Err(Error::NeedsReview { heads, .. }) if heads == [2, 3])
    );
    let read = lab.store.read_revision(MEMORY, 3, 20).unwrap();
    assert_eq!(read.bytes, b"branch b");
    assert_eq!(read.state, RevisionState::Unresolved);
    assert_eq!(
        lab.store.read_revision(MEMORY, 1, 1).unwrap().state,
        RevisionState::Historical
    );
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 3, b"no merge", metadata("a")),
        Err(Error::NeedsReview { .. })
    ));
    assert_eq!(lab.files().len(), 3);
}

#[test]
fn invalid_bindings_and_record_scope_never_broaden_access() {
    let lab = Lab::new();
    let common = project().common().clone();
    assert!(matches!(
        Binding::new(lab.root.clone(), &DocumentMetadata::Note(common), PROJECT),
        Err(Error::InvalidBinding)
    ));
    assert!(matches!(
        Binding::new(lab.root.clone(), &project(), MEMORY),
        Err(Error::InvalidBinding)
    ));
    assert!(matches!(
        Binding::new("relative".into(), &project(), PROJECT),
        Err(Error::InvalidBinding)
    ));
    let mut invalid = project();
    if let DocumentMetadata::Project(c) = &mut invalid {
        c.brain_version = 2;
    }
    assert!(matches!(
        Binding::new(lab.root.clone(), &invalid, PROJECT),
        Err(Error::InvalidBinding)
    ));
    assert!(!lab.memory.parent().unwrap().join(".writer.lock").exists());
    lab.create();
    let original = lab.header();
    for field in ["project", "memory", "collection"] {
        let mut h = original.clone();
        match field {
            "project" => h.project_id = MEMORY,
            "memory" => h.memory_id = PROJECT,
            _ => h.collection = "personal".into(),
        }
        lab.seed(1, h, ORIGINAL);
        assert!(matches!(
            lab.store.read_current(MEMORY, 1),
            Err(Error::InvalidBinding)
        ));
    }
    lab.seed(1, original, ORIGINAL);
    assert!(matches!(
        lab.store.read_current(PROJECT, 1),
        Err(Error::NotFound)
    ));
}

#[test]
fn root_parent_base_and_version_validation() {
    for variant in 0..7 {
        let lab = Lab::new();
        lab.create();
        let mut h = lab.header();
        let filename = if variant < 3 { 2 } else { 1 };
        match variant {
            0 => {
                h.revision = 2;
                h.parent = Some(999);
            }
            1 => {
                h.revision = 2;
                h.parent = Some(2);
            }
            2 => {
                h.revision = 2;
                h.parent = None;
            }
            3 => h.source_view = "../personal.md".into(),
            4 => h.base_revision = 2,
            5 => h.store_version = 2,
            _ => h.revision = 3,
        }
        lab.seed(filename, h, b"bad");
        assert!(matches!(
            lab.store.read_current(MEMORY, 1),
            Err(Error::Malformed(_)) | Err(Error::UnsupportedVersion(2))
        ));
    }
}

#[test]
fn framing_input_read_and_graph_limits_fail_closed() {
    let lab = Lab::new();
    lab.create();
    let original = fs::read(lab.revision(1)).unwrap();
    for bad in [
        vec![],
        b"BADMAGIC0000".to_vec(),
        [
            b"BRAINMR1".as_slice(),
            &((MAX_HEADER_BYTES + 1) as u32).to_le_bytes(),
        ]
        .concat(),
        original[..original.len() - 1].to_vec(),
    ] {
        fs::write(lab.revision(1), bad).unwrap();
        let result = lab.store.read_current(MEMORY, 1);
        assert!(
            matches!(result, Err(Error::Malformed(_)) | Err(Error::Limit(_))),
            "{result:?}"
        );
    }
    fs::write(lab.revision(1), &original).unwrap();
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, &vec![0; MAX_PAYLOAD_BYTES + 1], metadata("a")),
        Err(Error::Limit(_))
    ));
    let mut oversized_header = metadata("a");
    oversized_header.sources = (0..32)
        .map(|_| Source {
            reference: "s".repeat(2048),
            locator: None,
            revision: None,
        })
        .collect();
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, b"ok", oversized_header),
        Err(Error::Limit("header"))
    ));
    assert_eq!(lab.files(), vec![lab.revision(1)]); // Owned staging file cleaned on encode failure.
    assert_eq!(fs::read(lab.revision(1)).unwrap(), original);
    let mut h = lab.header();
    h.revision = MAX_REVISION;
    h.parent = Some(1);
    lab.seed(MAX_REVISION, h, b"last possible revision");
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, MAX_REVISION, b"overflow", metadata("a")),
        Err(Error::Overflow)
    ));
}

#[test]
fn finite_graph_count_bytes_and_directory_work() {
    let lab = Lab::new();
    lab.create();
    let mut h = lab.header();
    for n in 2..=MAX_REVISIONS as u64 {
        h.revision = n;
        h.parent = Some(n - 1);
        lab.seed(n, h.clone(), b"x");
    }
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, MAX_REVISIONS as u64, b"x", metadata("a")),
        Err(Error::Limit("revision count"))
    ));
    h.revision += 1;
    h.parent = Some(h.revision - 1);
    lab.seed(h.revision, h.clone(), b"x");
    assert!(matches!(
        lab.store.read_current(MEMORY, 1),
        Err(Error::Limit(_))
    ));
    let lab = Lab::new();
    lab.create();
    let mut h = lab.header();
    for n in 2..=9 {
        h.revision = n;
        h.parent = Some(n - 1);
        lab.seed(n, h.clone(), &vec![0; MAX_PAYLOAD_BYTES]);
    }
    assert!(matches!(
        lab.store.read_current(MEMORY, 1),
        Err(Error::Limit(_))
    ));
    let lab = Lab::new();
    lab.create();
    for n in 0..MAX_REVISIONS + 16 {
        fs::write(
            lab.memory.join("revisions").join(format!(".pending-{n}")),
            b"not adopted",
        )
        .unwrap();
    }
    assert!(matches!(
        lab.store.read_current(MEMORY, 1),
        Err(Error::Limit("directory entries"))
    ));
}

#[test]
fn isolated_staging_remnants_are_not_adopted_and_publication_reserves_room() {
    let lab = Lab::new();
    let dir = lab.memory.join("revisions");
    fs::create_dir(&dir).unwrap();
    for n in 0..MAX_REVISIONS + 16 {
        fs::write(dir.join(format!(".pending-{n}")), b"crash remnant").unwrap();
    }
    assert!(matches!(
        lab.store.create_from_view(MEMORY, metadata("a")),
        Err(Error::Limit("directory entries"))
    ));
    assert!(!lab.revision(1).exists());
    // Synthetic external recovery makes exactly one entry available.
    fs::remove_file(dir.join(".pending-0")).unwrap();
    lab.create();
    assert_eq!(
        lab.store
            .read_current(MEMORY, MAX_READ_BYTES)
            .unwrap()
            .bytes,
        ORIGINAL
    );
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, b"no room", metadata("a")),
        Err(Error::Limit("directory entries"))
    ));
    assert!(!lab.revision(2).exists());
    assert_eq!(fs::read(dir.join(".pending-1")).unwrap(), b"crash remnant");
}

#[cfg(unix)]
#[test]
fn observed_symlinks_nonregular_paths_and_traversal_are_rejected() {
    use std::os::unix::fs::symlink;
    let lab = Lab::new();
    let alias = lab.root.join("alias");
    symlink(&lab.memory, &alias).unwrap();
    assert!(matches!(
        Binding::new(alias, &project(), PROJECT),
        Err(Error::UnsafePath(_))
    ));
    assert!(matches!(
        Binding::new(lab.root.join(".."), &project(), PROJECT),
        Err(Error::UnsafePath(_))
    ));
    let view = lab.memory.join("view.md");
    fs::remove_file(&view).unwrap();
    symlink(lab.root.join("outside"), &view).unwrap();
    assert!(matches!(
        lab.store.create_from_view(MEMORY, metadata("a")),
        Err(Error::UnsafePath(_))
    ));
    fs::remove_file(&view).unwrap();
    fs::create_dir(&view).unwrap();
    assert!(matches!(
        lab.store.create_from_view(MEMORY, metadata("a")),
        Err(Error::UnsafePath(_))
    ));
    fs::remove_dir(&view).unwrap();
    fs::write(&view, ORIGINAL).unwrap();
    lab.create();
    symlink(&view, lab.memory.join("revisions/2.bmr")).unwrap();
    let result = lab.store.read_current(MEMORY, 1);
    assert!(matches!(result, Err(Error::UnsafePath(_))), "{result:?}");
}

struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Worker {
    fn wait_for(&mut self, marker: &str) {
        for _ in 0..32 {
            let mut line = String::new();
            assert!(
                self.output.read_line(&mut line).unwrap() > 0,
                "worker exited before {marker}"
            );
            if line.contains(marker) {
                return;
            }
        }
        panic!("worker protocol exceeded its bound");
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn separate_process_coordinates_busy_then_stale_without_hidden_retry() {
    let lab = Lab::new();
    lab.create();
    let lock_path = lab.memory.parent().unwrap().join(".writer.lock");
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&lock_path).unwrap().ino()
    };
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "writer_child", "--ignored", "--nocapture"])
        .env("BRAIN_H1_CHILD_ROOT", &lab.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut worker = Worker {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
    };
    worker.wait_for("H1_LOCKED");
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, b"parent", metadata("parent")),
        Err(Error::Busy)
    ));
    assert_eq!(lab.files().len(), 1);
    writeln!(worker.input, "publish").unwrap();
    worker.input.flush().unwrap();
    worker.wait_for("H1_PUBLISHED_2");
    assert!(worker.child.wait().unwrap().success());
    assert!(matches!(
        lab.store
            .publish_successor(MEMORY, 1, b"parent", metadata("parent")),
        Err(Error::RevisionConflict { actual: 2, .. })
    ));
    let read = lab.store.read_current(MEMORY, 10).unwrap();
    assert_eq!(read.header.metadata.session, "child");
    assert_eq!(read.bytes, b"child");
    assert_eq!(lab.files().len(), 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(lock_path).unwrap().ino(), inode);
    }
}

#[test]
#[ignore = "IPC subprocess entry point, executed by the coordination test"]
fn writer_child() {
    let root = PathBuf::from(std::env::var_os("BRAIN_H1_CHILD_ROOT").expect("fixture root"));
    let lock_path = root
        .join(PROJECT.to_string())
        .join(COLLECTION)
        .join(".writer.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    file.try_lock().unwrap();
    println!("H1_LOCKED");
    std::io::stdout().flush().unwrap();
    let mut command = String::new();
    std::io::stdin().read_line(&mut command).unwrap();
    assert_eq!(command.trim(), "publish");
    drop(file);
    let result = store(&root)
        .publish_successor(MEMORY, 1, b"child", metadata("child"))
        .unwrap();
    println!("H1_PUBLISHED_{}", result.revision);
    std::io::stdout().flush().unwrap();
}
