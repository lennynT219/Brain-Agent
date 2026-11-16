use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Component, Path},
};
use tempfile::PersistError;

const MAGIC: &[u8; 8] = b"BRAINMR1";
const PREFIX: usize = 12;
const MAX_RECORD: usize = PREFIX + MAX_HEADER_BYTES + MAX_PAYLOAD_BYTES;
const STAGING: &str = ".pending-";
const MAX_DIRECTORY_ENTRIES: usize = MAX_REVISIONS + 16;

// Leave room for the new final record as well as isolated crash leftovers.
fn publication_room(dir: &Path) -> Result<()> {
    for (index, entry) in fs::read_dir(dir)?.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES - 1 {
            return Err(Error::Limit("directory entries"));
        }
        entry?;
    }
    Ok(())
}

/// Checks observed paths; trusted stable ancestors are still required (no CAS).
fn inspect(path: &Path, is_dir: bool) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::UnsafePath(path.into()));
    }
    let mut walked = PathBuf::new();
    for part in path.components() {
        if !matches!(part, Component::RootDir | Component::Normal(_)) {
            return Err(Error::UnsafePath(path.into()));
        }
        walked.push(part);
        let metadata = fs::symlink_metadata(&walked)?;
        let last = walked == path;
        if metadata.file_type().is_symlink()
            || if last && !is_dir {
                !metadata.is_file()
            } else {
                !metadata.is_dir()
            }
        {
            return Err(Error::UnsafePath(walked));
        }
    }
    Ok(())
}

pub(super) fn directory(path: &Path) -> Result<()> {
    inspect(path, true)
}

fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    inspect(path, false)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(Error::Limit("file bytes"));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Limit("file bytes"));
    }
    Ok(bytes)
}

struct Record {
    header: Header,
    bytes: Vec<u8>,
}

impl Record {
    fn decode(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < PREFIX || &bytes[..8] != MAGIC {
            return Err(Error::Malformed("magic/prefix"));
        }
        let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if size > MAX_HEADER_BYTES {
            return Err(Error::Limit("header"));
        }
        let end = PREFIX.checked_add(size).ok_or(Error::Overflow)?;
        let text = std::str::from_utf8(
            bytes
                .get(PREFIX..end)
                .ok_or(Error::Malformed("header length"))?,
        )
        .map_err(|_| Error::Malformed("header UTF-8"))?;
        let header: Header = toml::from_str(text).map_err(|_| Error::Malformed("header TOML"))?;
        if header.store_version != STORE_VERSION {
            return Err(Error::UnsupportedVersion(header.store_version));
        }
        header.metadata.validate()?;
        let payload = &bytes[end..];
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::Limit("payload"));
        }
        if header.payload_len != payload.len() as u64 {
            return Err(Error::Malformed("payload length/trailing bytes"));
        }
        Ok(Self {
            header,
            bytes: payload.to_vec(),
        })
    }

    fn encode(&self) -> Result<Vec<u8>> {
        if self.bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::Limit("payload"));
        }
        self.header.metadata.validate()?;
        let text = toml::to_string(&self.header).map_err(|_| Error::InvalidInput("header TOML"))?;
        if text.len() > MAX_HEADER_BYTES {
            return Err(Error::Limit("header"));
        }
        let mut bytes = Vec::with_capacity(PREFIX + text.len() + self.bytes.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(&self.bytes);
        Ok(bytes)
    }
}

struct Graph {
    records: BTreeMap<u64, Record>,
    heads: Vec<u64>,
    total_bytes: usize,
}

impl Graph {
    fn view(&self, path: &Path) -> Result<ViewState> {
        let view = bounded_read(path, MAX_PAYLOAD_BYTES)?;
        if view != self.records[&1].bytes {
            Ok(ViewState::Diverged)
        } else if self.heads == [1] {
            Ok(ViewState::Base)
        } else {
            Ok(ViewState::Stale)
        }
    }

    fn resolved(&self, view: ViewState) -> Result<u64> {
        if self.heads.len() != 1 || view == ViewState::Diverged {
            return Err(Error::NeedsReview {
                heads: self.heads.clone(),
                view,
            });
        }
        Ok(self.heads[0])
    }
}

// Explicit unlock also releases an inherited open-file-description lock during
// another thread's fork/exec window; closing only our fd can leave it held there.
struct WriterLock(File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Debug, Clone)]
pub struct MemoryStore {
    binding: Binding,
}

impl MemoryStore {
    pub fn new(binding: Binding) -> Self {
        Self { binding }
    }

    fn collection(&self) -> PathBuf {
        self.binding
            .root
            .join(self.binding.project_id.to_string())
            .join(COLLECTION)
    }

    fn memory(&self, id: Uuid) -> PathBuf {
        self.collection().join(id.to_string())
    }

    /// Lock file is persistent: deleting it would split the coordination domain.
    fn lock(&self) -> Result<WriterLock> {
        let collection = self.collection();
        directory(&collection)?;
        let path = collection.join(".writer.lock");
        match fs::symlink_metadata(&path) {
            Ok(_) => inspect(&path, false)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(WriterLock(file)),
            Err(TryLockError::WouldBlock) => Err(Error::Busy),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }

    fn load(&self, id: Uuid) -> Result<Graph> {
        let dir = self.memory(id).join("revisions");
        directory(&dir)?;
        let mut records = BTreeMap::new();
        let mut total = 0usize;
        for (index, entry) in fs::read_dir(&dir)?.enumerate() {
            if index >= MAX_DIRECTORY_ENTRIES {
                return Err(Error::Limit("directory entries"));
            }
            let entry = entry?;
            inspect(&entry.path(), false)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Malformed("filename"))?;
            if name.starts_with(STAGING) {
                continue; // Isolated crash leftovers; never adopt or delete them.
            }
            let number = name
                .strip_suffix(".bmr")
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0 && name == format!("{n}.bmr"))
                .ok_or(Error::Malformed("revision filename"))?;
            if records.len() >= MAX_REVISIONS {
                return Err(Error::Limit("revision count"));
            }
            let remaining = MAX_GRAPH_BYTES
                .checked_sub(total)
                .ok_or(Error::Limit("graph bytes"))?;
            let bytes = bounded_read(&entry.path(), MAX_RECORD.min(remaining))?;
            total = total.checked_add(bytes.len()).ok_or(Error::Overflow)?;
            let record = Record::decode(bytes)?;
            let h = &record.header;
            if h.project_id != self.binding.project_id
                || h.collection != COLLECTION
                || h.memory_id != id
            {
                return Err(Error::InvalidBinding);
            }
            if h.revision != number
                || h.revision > MAX_REVISION
                || h.base_revision != 1
                || h.source_view != "view.md"
            {
                return Err(Error::Malformed("revision/base identity"));
            }
            records.insert(number, record);
        }
        if records.is_empty() {
            return Err(Error::NotFound);
        }
        let mut heads: BTreeSet<u64> = records.keys().copied().collect();
        for (number, record) in &records {
            match record.header.parent {
                None if *number == 1 => (),
                Some(parent) if parent < *number && records.contains_key(&parent) => {
                    heads.remove(&parent);
                }
                _ => return Err(Error::Malformed("root/parent graph")),
            }
        }
        if !records.contains_key(&1) {
            return Err(Error::Malformed("missing root"));
        }
        Ok(Graph {
            records,
            heads: heads.into_iter().collect(),
            total_bytes: total,
        })
    }

    pub fn create_from_view(&self, id: Uuid, metadata: Metadata) -> Result<Publication> {
        metadata.validate()?;
        let _lock = self.lock()?;
        let memory = self.memory(id);
        directory(&memory)?;
        let dir = memory.join("revisions");
        match fs::symlink_metadata(&dir) {
            Ok(_) => {
                directory(&dir)?;
                match self.load(id) {
                    Ok(_) => return Err(Error::AlreadyExists),
                    Err(Error::NotFound) => (),
                    Err(e) => return Err(e),
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&dir)?,
            Err(e) => return Err(e.into()),
        }
        let view = memory.join("view.md");
        let bytes = bounded_read(&view, MAX_PAYLOAD_BYTES)?;
        let record = self.record(id, 1, None, bytes.clone(), metadata);
        publication_room(&dir)?;
        self.publish(&dir, &record, || {
            File::open(&dir)?.sync_all()?;
            Ok(if bounded_read(&view, MAX_PAYLOAD_BYTES)? == bytes {
                ViewState::Base
            } else {
                ViewState::Diverged
            })
        })
    }

    pub fn publish_successor(
        &self,
        id: Uuid,
        expected: u64,
        bytes: &[u8],
        metadata: Metadata,
    ) -> Result<Publication> {
        if expected == 0 || expected > MAX_REVISION {
            return Err(Error::InvalidInput("expected revision"));
        }
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::Limit("payload"));
        }
        metadata.validate()?;
        let _lock = self.lock()?;
        let graph = self.load(id)?;
        let view = self.memory(id).join("view.md");
        let actual = graph.resolved(graph.view(&view)?)?;
        if actual != expected {
            return Err(Error::RevisionConflict { expected, actual });
        }
        if graph.records.len() >= MAX_REVISIONS {
            return Err(Error::Limit("revision count"));
        }
        let revision = actual
            .checked_add(1)
            .filter(|n| *n <= MAX_REVISION)
            .ok_or(Error::Overflow)?;
        let record = self.record(id, revision, Some(actual), bytes.to_vec(), metadata);
        let dir = self.memory(id).join("revisions");
        if graph
            .total_bytes
            .checked_add(record.encode()?.len())
            .ok_or(Error::Overflow)?
            > MAX_GRAPH_BYTES
        {
            return Err(Error::Limit("graph bytes"));
        }
        publication_room(&dir)?;
        self.publish(&dir, &record, || {
            File::open(&dir)?.sync_all()?;
            Ok(
                if bounded_read(&view, MAX_PAYLOAD_BYTES)? == graph.records[&1].bytes {
                    ViewState::Stale
                } else {
                    ViewState::Diverged
                },
            )
        })
    }

    fn record(
        &self,
        id: Uuid,
        revision: u64,
        parent: Option<u64>,
        bytes: Vec<u8>,
        metadata: Metadata,
    ) -> Record {
        Record {
            header: Header {
                store_version: STORE_VERSION,
                project_id: self.binding.project_id,
                collection: COLLECTION.into(),
                memory_id: id,
                revision,
                parent,
                source_view: "view.md".into(),
                base_revision: 1,
                payload_len: bytes.len() as u64,
                metadata,
            },
            bytes,
        }
    }

    fn publish(
        &self,
        dir: &Path,
        record: &Record,
        after: impl FnOnce() -> Result<ViewState>,
    ) -> Result<Publication> {
        let mut staged = tempfile::Builder::new().prefix(STAGING).tempfile_in(dir)?;
        staged.write_all(&record.encode()?)?;
        staged.flush()?;
        staged.as_file().sync_all()?;
        let target = dir.join(format!("{}.bmr", record.header.revision));
        // Regular-file collision is handled by no-clobber; reject other observed leaves first.
        match fs::symlink_metadata(&target) {
            Ok(_) => {
                inspect(&target, false)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        finish_publication(
            staged.persist_noclobber(target),
            record.header.revision,
            after,
        )
    }

    pub fn read_current(&self, id: Uuid, max_bytes: usize) -> Result<ReadResult> {
        self.read(id, None, max_bytes)
    }

    pub fn read_revision(&self, id: Uuid, revision: u64, max_bytes: usize) -> Result<ReadResult> {
        if revision == 0 || revision > MAX_REVISION {
            return Err(Error::InvalidInput("revision"));
        }
        self.read(id, Some(revision), max_bytes)
    }

    fn read(&self, id: Uuid, revision: Option<u64>, max_bytes: usize) -> Result<ReadResult> {
        if max_bytes > MAX_READ_BYTES {
            return Err(Error::Limit("read result"));
        }
        let _lock = self.lock()?;
        let graph = self.load(id)?;
        let view = graph.view(&self.memory(id).join("view.md"))?;
        let revision = match revision {
            Some(r) => r,
            None => graph.resolved(view)?,
        };
        let record = graph.records.get(&revision).ok_or(Error::NotFound)?;
        let needs_review = graph.heads.len() != 1 || view == ViewState::Diverged;
        let state = if !graph.heads.contains(&revision) {
            RevisionState::Historical
        } else if needs_review {
            RevisionState::Unresolved
        } else {
            RevisionState::Current
        };
        Ok(ReadResult {
            header: record.header.clone(),
            bytes: record.bytes[..max_bytes.min(record.bytes.len())].to_vec(),
            total_bytes: record.bytes.len(),
            truncated: max_bytes < record.bytes.len(),
            heads: graph.heads,
            state,
            view,
            needs_review,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{CommonMetadata, DOCUMENT_VERSION};

    fn fixture() -> (tempfile::TempDir, MemoryStore, Uuid, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let project_id = Uuid::from_u128(1);
        let id = Uuid::from_u128(2);
        let project = DocumentMetadata::Project(CommonMetadata {
            brain_version: DOCUMENT_VERSION,
            brain_id: project_id,
            title: "Fixture".into(),
        });
        let store =
            MemoryStore::new(Binding::new(temp.path().into(), &project, project_id).unwrap());
        let memory = store.memory(id);
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("view.md"), b"original").unwrap();
        store.create_from_view(id, metadata()).unwrap();
        (temp, store, id, memory.join("revisions"))
    }
    fn metadata() -> Metadata {
        Metadata {
            title: "Fixture".into(),
            claim_kind: ClaimKind::SourceObservation,
            session: "synthetic-session".into(),
            observed_at_unix_ms: 0,
            sources: vec![],
        }
    }
    fn successor(store: &MemoryStore, id: Uuid) -> Record {
        store.record(id, 2, Some(1), b"successor".to_vec(), metadata())
    }
    fn no_staging(dir: &Path) {
        assert!(fs::read_dir(dir).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(STAGING)
        }));
    }

    #[test]
    fn collision_never_calls_postcommit_or_removes_another_record() {
        let (_temp, store, id, dir) = fixture();
        let record = successor(&store, id);
        let target = dir.join("2.bmr");
        fs::write(&target, b"another writer's bytes").unwrap();
        let result = store.publish(&dir, &record, || panic!("collision cannot commit"));
        assert!(matches!(result, Err(Error::Collision(2))));
        assert_eq!(fs::read(target).unwrap(), b"another writer's bytes");
        no_staging(&dir);
    }

    #[test]
    fn postcommit_failure_keeps_valid_unacknowledged_successor() {
        let (_temp, store, id, dir) = fixture();
        let record = successor(&store, id);
        let result = store.publish(&dir, &record, || {
            Err(Error::Io(io::Error::other(
                "injected sync/observation failure",
            )))
        });
        assert!(matches!(
            result,
            Err(Error::AfterPublication { revision: 2, .. })
        ));
        let current = store.read_current(id, 100).unwrap();
        assert_eq!(current.header.revision, 2);
        assert_eq!(current.bytes, b"successor");
        assert_eq!(store.read_revision(id, 1, 100).unwrap().bytes, b"original");
        no_staging(&dir);
    }

    #[test]
    fn uncertain_persist_result_does_not_delete_a_published_link() {
        let (_temp, store, id, dir) = fixture();
        let mut staged = tempfile::Builder::new()
            .prefix(STAGING)
            .tempfile_in(&dir)
            .unwrap();
        staged
            .write_all(&successor(&store, id).encode().unwrap())
            .unwrap();
        // Model link publication followed by a failure before persist acknowledges it.
        fs::hard_link(staged.path(), dir.join("2.bmr")).unwrap();
        let failure = PersistError {
            error: io::Error::other("injected post-link failure"),
            file: staged,
        };
        let result = finish_publication(Err(failure), 2, || {
            panic!("uncertain result is not success")
        });
        assert!(matches!(
            result,
            Err(Error::PublicationUncertain { revision: 2, .. })
        ));
        assert_eq!(store.read_current(id, 100).unwrap().header.revision, 2);
        no_staging(&dir);
    }

    #[test]
    fn postcommit_manual_drift_flags_review_without_rollback() {
        let (_temp, store, id, dir) = fixture();
        let view = store.memory(id).join("view.md");
        let graph = store.load(id).unwrap();
        let result = store
            .publish(&dir, &successor(&store, id), || {
                fs::write(&view, b"human bytes after publication")?;
                graph.view(&view)
            })
            .unwrap();
        assert!(result.needs_review);
        assert_eq!(result.view, ViewState::Diverged);
        assert!(dir.join("2.bmr").exists());
        assert_eq!(fs::read(view).unwrap(), b"human bytes after publication");
    }

    #[test]
    fn explicit_unlock_releases_a_cloned_open_file_description() {
        let (_temp, store, _id, _dir) = fixture();
        let guard = store.lock().unwrap();
        let inherited_fd = guard.0.try_clone().unwrap();
        drop(guard);
        let _next = store.lock().unwrap();
        drop(inherited_fd);
    }
}

fn finish_publication(
    result: std::result::Result<File, PersistError>,
    revision: u64,
    after: impl FnOnce() -> Result<ViewState>,
) -> Result<Publication> {
    match result {
        Ok(file) => drop(file), // Namespace publication is the commit point, not later observation.
        Err(failure) => {
            let PersistError { error, file } = failure;
            drop(file); // Only our temporary path; never remove the destination.
            return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                Error::Collision(revision)
            } else {
                Error::PublicationUncertain {
                    revision,
                    source: error,
                }
            });
        }
    }
    let view = after().map_err(|source| Error::AfterPublication {
        revision,
        source: Box::new(source),
    })?;
    Ok(Publication {
        revision,
        view,
        needs_review: view == ViewState::Diverged,
    })
}
