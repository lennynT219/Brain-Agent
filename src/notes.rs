//! The host owns cwd and Brain's existing vault configuration. This endpoint
//! resolves exactly one Project directory and never consults Memory bindings.
use crate::document::{CommonMetadata, DOCUMENT_VERSION, DocumentMetadata};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const LOCK: &str = ".brain-notes.lock";
const RENAME_INTENT: &str = ".brain-note-rename.json";
const MAX_INTENT: usize = 12 * MAX_DOCUMENT + 4096;
const MAX_ENTRIES: usize = 256;
const MAX_DOCUMENT: usize = 512 * 1024;
const MAX_TITLE: usize = 256;

#[derive(Debug)]
pub(crate) struct Failure(pub(crate) &'static str, pub(crate) &'static str);
type Result<T> = std::result::Result<T, Failure>;

fn project_failure(error: crate::memory::store::Failure) -> Failure {
    let status = match error.status.as_str() {
        "invalid" => "invalid", "unbound" => "unbound", "conflict" => "conflict",
        "not_found" => "not_found", "busy" => "busy", "denied" => "denied", _ => "unavailable",
    };
    Failure(status, "Project binding or selected Project is unavailable")
}

fn unavailable(message: &'static str) -> Failure {
    Failure("unavailable", message)
}

pub(crate) fn validate_id(id: Uuid) -> Result<()> {
    if id.is_nil() {
        return Err(Failure("invalid", "identity must be a nonnil UUID"));
    }
    Ok(())
}

fn valid_title(title: &str) -> bool {
    !title.trim().is_empty() && title.len() <= MAX_TITLE && !title.chars().any(char::is_control)
}

// Creation and explicit rename only: a legacy note is still addressed by brain_id, even if its
// preexisting filename or title would not meet the new naming rules.
fn named_filename(title: &str) -> Result<String> {
    let filename = format!("{title}.md");
    if !valid_title(title)
        || title.trim() != title
        || title.starts_with('.') // Obsidian ignores dotfiles.
        || title.ends_with('.')
        || filename.len() > 255
        || filename.encode_utf16().count() > 255
        || !crate::project_binding::safe_component(&filename)
        || crate::project_binding::reserved_windows_stem(title)
    {
        return Err(Failure("invalid", "Note title must form a portable Markdown filename"));
    }
    Ok(filename)
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Binding {
    binding_version: u32,
    project_id: Uuid,
    project_path: String,
    #[serde(default)]
    allow_write: bool,
}

#[derive(PartialEq, Eq)]
pub(crate) struct Scope {
    binding: Binding,
    notes: PathBuf,
}

struct Note {
    metadata: CommonMetadata,
    path: PathBuf,
    markdown: String,
    body_start: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameIntent {
    version: u32,
    project_id: Uuid,
    note_id: Uuid,
    source: String,
    destination: String,
    before: String,
    after: String,
}

fn rename_partial() -> Failure {
    Failure("partial", "Note rename requires inspection; do not retry")
}

fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(unavailable("cannot inspect Note publication path")),
    }
}

fn note_filename(name: &str) -> bool {
    let path = Path::new(name);
    !name.contains(['/', '\\', ':']) && path.file_name().and_then(|name| name.to_str()) == Some(name)
        && path.extension().is_some_and(|ext| ext == "md")
}

fn revised_title(note: &Note, title: &str) -> Result<String> {
    let invalid = || Failure("invalid", "Note title cannot be replaced without rewriting unrelated YAML");
    let mut lines = note.markdown[..note.body_start].split_inclusive('\n');
    let mut offset = 0;
    let mut range = None;
    while let Some(line) = lines.next() {
        let bare = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = bare.strip_prefix("title:") {
            let scalar = value.trim_matches([' ', '\t']);
            let simple = (scalar == note.metadata.title
                && !scalar.starts_with(['&', '*', '!', '|', '>', '\'', '"', '[', '{'])
                && !scalar.contains('#'))
                || serde_json::from_str::<String>(scalar).is_ok_and(|s| s == note.metadata.title);
            // A later indented continuation, even past a comment/blank line,
            // must not become detached from a replaced scalar.
            let continuation = lines.clone()
                .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
                .is_some_and(|line| line.starts_with([' ', '\t']));
            if !simple || continuation || range.is_some() {
                return Err(invalid());
            }
            let start = offset + bare.len() - value.trim_start_matches([' ', '\t']).len();
            range = Some(start..start + scalar.len());
        }
        offset += line.len();
    }
    let mut next = note.markdown.clone();
    let range = range.ok_or_else(invalid)?;
    let encoded = serde_json::to_string(title).map_err(|_| invalid())?;
    next.replace_range(range, &encoded);
    let mut expected = note.metadata.clone();
    expected.title = title.into();
    if !matches!(metadata(&next, "note"), Ok(Some((DocumentMetadata::Note(decoded), body))) if decoded == expected && next[body..] == note.markdown[note.body_start..]) {
        return Err(invalid());
    }
    Ok(next)
}

fn ensure_directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return Err(unavailable(
            "configured directory must exist and not be a symlink",
        ));
    }
    Ok(())
}

fn is_regular(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return false;
        }
    }
    metadata.is_file()
}

pub(crate) fn read_regular(path: &Path, limit: usize) -> Result<String> {
    // This is a local, trusted-directory boundary, not an OS sandbox against
    // concurrent substitution of directory entries by an untrusted process.
    if !fs::symlink_metadata(path).is_ok_and(|m| is_regular(&m)) {
        return Err(unavailable("expected a regular, unaliased file"));
    }
    let mut bytes = Vec::new();
    let file = File::open(path).map_err(|_| unavailable("cannot open selected file"))?;
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable("cannot read selected file"))?;
    if bytes.len() > limit {
        return Err(unavailable("selected file exceeds byte limit"));
    }
    String::from_utf8(bytes).map_err(|_| unavailable("selected file is not UTF-8"))
}

#[derive(Deserialize)]
struct HeaderKind {
    brain_type: Option<String>,
}

/// Ignore unmanaged Markdown and other typed documents. A malformed header is
/// not silently dropped: it may contain a duplicate identity or another Project.
pub(crate) fn metadata(markdown: &str, kind: &str) -> Result<Option<(DocumentMetadata, usize)>> {
    let Some(tail) = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
    else {
        return Ok(None);
    };
    let mut end = 0;
    for line in tail.split_inclusive('\n') {
        if matches!(line, "---" | "---\n" | "---\r\n") {
            let yaml = &tail[..end];
            let header: Option<HeaderKind> = serde_saphyr::from_str(yaml)
                .map_err(|_| unavailable("malformed document frontmatter"))?;
            if header.as_ref().and_then(|h| h.brain_type.as_deref()) != Some(kind) {
                return Ok(None);
            }
            let metadata: DocumentMetadata = serde_saphyr::from_str(yaml)
                .map_err(|_| unavailable("invalid managed document metadata"))?;
            metadata.validate().map_err(unavailable)?;
            if metadata.common().brain_id.is_nil() {
                return Err(unavailable("managed document identity is nil"));
            }
            let body_start = markdown.len() - tail.len() + end + line.len();
            return Ok(Some((metadata, body_start)));
        }
        end += line.len();
    }
    Err(unavailable("unterminated document frontmatter"))
}

pub(crate) fn direct_markdown(dir: &Path) -> Result<(Vec<PathBuf>, usize)> {
    let mut files = Vec::new();
    let mut entries = 0;
    for entry in
        fs::read_dir(dir).map_err(|_| unavailable("cannot enumerate selected directory"))?
    {
        let entry = entry.map_err(|_| unavailable("cannot enumerate directory entry"))?;
        if entry.file_name() == LOCK {
            continue;
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(unavailable("selected directory exceeds entry limit"));
        }
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|_| unavailable("cannot inspect Markdown entry"))?;
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(unavailable("Markdown entry is not a regular file"));
        }
        files.push(path);
    }
    files.sort_unstable();
    Ok((files, entries))
}

impl Scope {
    pub(crate) fn load() -> Result<Self> {
        let principal = crate::projects::notes_principal().map_err(project_failure)?
            .ok_or(Failure("unbound", "no Project binding in the host working directory"))?;
        Self::load_binding(Binding { binding_version: 1, project_id: principal.project_id,
            project_path: principal.project_path, allow_write: principal.allow_write })
    }

    /// Explicit additional Project selection never enables writes.
    pub(crate) fn load_additional(project_id: Uuid) -> Result<Self> {
        let vault = crate::projects::vault().map_err(project_failure)?;
        let project = crate::projects::find(&vault, project_id).map_err(project_failure)?;
        Self::load_binding(Binding { binding_version: 1, project_id, project_path: project.path, allow_write: false })
    }

    fn load_binding(binding: Binding) -> Result<Self> {
        let parts: Vec<_> = binding.project_path.split('/').collect();
        if binding.binding_version != 1
            || binding.project_id.is_nil()
            || parts.len() != 3
            || parts[0] != "01-proyectos"
            || !parts[2].ends_with(".md")
            || parts.iter().any(|part| {
                part.trim().is_empty()
                    || part.contains(['\\', ':', '\0'])
                    || !matches!(
                        Path::new(part).components().next(),
                        Some(Component::Normal(_))
                    )
                    || matches!(*part, "." | "..")
            })
        {
            return Err(unavailable("invalid Notes Project binding"));
        }
        let config = crate::config::load_config()
            .map_err(|_| unavailable("configured vault is unavailable"))?;
        ensure_directory(&config.vault_path)?;
        let vault = config
            .vault_path
            .canonicalize()
            .map_err(|_| unavailable("cannot resolve configured vault"))?;
        let active = crate::projects::find(&vault, binding.project_id).map_err(project_failure)?;
        if active.path != binding.project_path { return Err(Failure("conflict", "Project binding path changed")); }
        let projects = vault.join(parts[0]);
        ensure_directory(&projects)?;
        let project_dir = projects.join(parts[1]);
        ensure_directory(&project_dir)?;
        let selected = project_dir.join(parts[2]);
        let mut matched = false;
        for path in direct_markdown(&project_dir)?.0 {
            let raw = read_regular(&path, MAX_DOCUMENT)?;
            if let Some((project, _)) = metadata(&raw, "project")? {
                if matched || path != selected || project.common().brain_id != binding.project_id {
                    return Err(unavailable("Project identity or directory is ambiguous"));
                }
                matched = true;
            }
        }
        if !matched {
            return Err(unavailable(
                "bound Project does not exist or is not a Project",
            ));
        }
        let notes = project_dir.join("notas");
        ensure_directory(&notes)?;
        Ok(Self { binding, notes })
    }

    fn notes(&self, mut visit: impl FnMut(Note)) -> Result<usize> {
        self.no_rename_intent()?;
        let (files, entries) = direct_markdown(&self.notes)?;
        let mut ids = HashSet::new();
        for path in files {
            let markdown = read_regular(&path, MAX_DOCUMENT)?;
            let Some((DocumentMetadata::Note(metadata), body_start)) = metadata(&markdown, "note")?
            else {
                continue;
            };
            if !valid_title(&metadata.title) {
                return Err(unavailable("invalid managed Note title"));
            }
            if !ids.insert(metadata.brain_id) {
                return Err(Failure(
                    "conflict",
                    "duplicate Note identity in authorized Project",
                ));
            }
            visit(Note {
                metadata,
                path,
                markdown,
                body_start,
            });
        }
        self.no_rename_intent()?;
        Ok(entries)
    }

    fn no_rename_intent(&self) -> Result<()> {
        if absent(&self.notes.join(RENAME_INTENT))? { Ok(()) } else { Err(rename_partial()) }
    }

    fn find(&self, note_id: Uuid) -> Result<Note> {
        let mut found = None;
        self.notes(|note| {
            if note.metadata.brain_id == note_id {
                found = Some(note);
            }
        })?;
        found.ok_or(Failure("not_found", "Note not found in authorized Project"))
    }

    // Before the first writer, readers do not create a lock. If a writer
    // appears mid-read, discard that observation instead of returning it.
    fn read_consistently(&self, work: impl FnOnce() -> Result<Value>) -> Result<Value> {
        let lock_path = self.notes.join(LOCK);
        if absent(&lock_path)? {
            let result = work();
            if !absent(&lock_path)? {
                return Err(Failure("busy", "Notes publication lock appeared during read"));
            }
            return result;
        }
        let _lock = self.lock_for_read(&lock_path)?;
        work()
    }

    fn lock_for_read(&self, path: &Path) -> Result<File> {
        if !fs::symlink_metadata(path).is_ok_and(|m| is_regular(&m)) {
            return Err(unavailable("invalid Notes publication lock"));
        }
        let lock = File::open(path).map_err(|_| unavailable("cannot open Notes publication lock"))?;
        match lock.try_lock_shared() {
            Ok(()) => Ok(lock),
            Err(fs::TryLockError::WouldBlock) => Err(Failure("busy", "Notes publication lock is held")),
            Err(fs::TryLockError::Error(_)) => Err(unavailable("cannot lock Notes for reading")),
        }
    }

    pub(crate) fn list(&self) -> Result<Value> {
        self.read_consistently(|| self.list_locked())
    }

    fn list_locked(&self) -> Result<Value> {
        let mut notes = Vec::new();
        self.notes(|note| {
            notes.push(json!({"note_id": note.metadata.brain_id, "title": note.metadata.title}))
        })?;
        Ok(json!({"status": "listed", "project_id": self.binding.project_id, "notes": notes}))
    }

    pub(crate) fn read(&self, note_id: Uuid) -> Result<Value> {
        self.read_consistently(|| self.read_locked(note_id))
    }

    fn read_locked(&self, note_id: Uuid) -> Result<Value> {
        let note = self.find(note_id)?;
        Ok(
            json!({"status": "found", "project_id": self.binding.project_id,
            "note_id": note_id, "title": note.metadata.title,
            "markdown": note.markdown, "body": &note.markdown[note.body_start..]}),
        )
    }

    fn lock_for_write(&self, expected_project: Uuid) -> Result<File> {
        if expected_project != self.binding.project_id {
            return Err(Failure("conflict", "active Project changed"));
        }
        if !self.binding.allow_write {
            return Err(Failure(
                "denied",
                "writes are disabled in this Notes binding",
            ));
        }
        let path = self.notes.join(LOCK);
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        let lock = match options.clone().create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&path).is_ok_and(|m| is_regular(&m)) {
                    return Err(unavailable("invalid Notes publication lock"));
                }
                options
                    .open(&path)
                    .map_err(|_| unavailable("cannot open Notes publication lock"))?
            }
            Err(_) => return Err(unavailable("cannot create Notes publication lock")),
        };
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(Failure("busy", "Notes publication lock is held"));
            }
            Err(fs::TryLockError::Error(_)) => {
                return Err(unavailable("cannot lock Notes for publication"));
            }
        }
        self.recheck()?;
        Ok(lock)
    }

    fn recheck(&self) -> Result<()> {
        match Self::load() {
            Ok(current) if &current == self => Ok(()),
            _ => Err(Failure(
                "conflict",
                "Notes authorization changed during the operation",
            )),
        }
    }

    pub(crate) fn create(&self, project_id: Uuid, title: String, body: &str) -> Result<Value> {
        let filename = named_filename(&title)?;
        let _lock = self.lock_for_write(project_id)?;
        // Avoid making a 256-entry directory unreadable by creating entry 257.
        let note_id = Uuid::new_v4();
        let mut id_taken = false;
        let mut title_taken = false;
        let folded_title = title.to_lowercase();
        if self.notes(|note| {
            id_taken |= note.metadata.brain_id == note_id;
            title_taken |= note.metadata.title.to_lowercase() == folded_title;
        })? >= MAX_ENTRIES {
            return Err(unavailable("Notes directory has reached the entry limit"));
        }
        if id_taken {
            return Err(Failure("conflict", "generated Note identity is already used"));
        }
        if title_taken {
            return Err(Failure("conflict", "a Note already uses this title in the Project"));
        }
        let metadata = DocumentMetadata::Note(CommonMetadata {
            brain_version: DOCUMENT_VERSION,
            brain_id: note_id,
            title: title.clone(),
        });
        let yaml = serde_saphyr::to_string(&metadata)
            .map_err(|_| Failure("invalid", "cannot encode Note metadata"))?;
        // Scope already validated the exact Project path. A vault-relative
        // wikilink disambiguates projects with the same filename. Keep it in a
        // quoted property so the author-supplied body stays byte-for-byte intact.
        let target = self.binding.project_path.strip_suffix(".md")
            .ok_or_else(|| unavailable("invalid Project link target"))?;
        let project_link = serde_json::to_string(&format!("[[{target}]]"))
            .map_err(|_| Failure("invalid", "cannot encode Project link"))?;
        let markdown = format!("---\n{}\nproject_link: {project_link}\n---\n{body}", yaml.trim_end_matches('\n'));
        // Do not publish a title that the Markdown codec cannot round-trip.
        if !matches!(self::metadata(&markdown, "note"), Ok(Some((decoded, _))) if decoded == metadata)
        {
            return Err(Failure("invalid", "cannot round-trip Note metadata"));
        }
        let staged = self.stage(&markdown, None)?;
        self.recheck()?;
        staged
            .persist_noclobber(self.notes.join(filename))
            .map_err(|e| {
                if e.error.kind() == io::ErrorKind::AlreadyExists {
                    Failure("conflict", "Note filename already exists")
                } else {
                    unavailable("cannot publish new Note")
                }
            })?;
        Ok(
            json!({"status": "created", "project_id": project_id, "note_id": note_id, "title": title}),
        )
    }

    pub(crate) fn update(
        &self,
        project_id: Uuid,
        note_id: Uuid,
        expected: &str,
        body: &str,
    ) -> Result<Value> {
        let _lock = self.lock_for_write(project_id)?;
        let note = self.find(note_id)?;
        if note.markdown != expected {
            return Err(Failure("conflict", "Note baseline no longer matches"));
        }
        let receipt = |status| json!({"status": status, "project_id": project_id, "note_id": note_id, "title": note.metadata.title});
        let original_body = &note.markdown[note.body_start..];
        if original_body == body {
            return Ok(receipt("unchanged"));
        }
        let mut markdown = note.markdown[..note.body_start].to_owned();
        if !markdown.ends_with('\n') {
            // Adding a body to a document whose closing delimiter is at EOF.
            markdown.push('\n');
        }
        markdown.push_str(body);
        let permissions = fs::symlink_metadata(&note.path)
            .map_err(|_| unavailable("cannot inspect Note permissions"))?
            .permissions();
        let staged = self.stage(&markdown, Some(permissions))?;
        self.recheck()?;
        if !matches!(read_regular(&note.path, MAX_DOCUMENT), Ok(current) if current == expected) {
            return Err(Failure("conflict", "Note changed before publication"));
        }
        // Atomic publication on the same filesystem. Only cooperating Brain
        // writers hold this lock; other editors must not write concurrently.
        staged
            .persist(&note.path)
            .map_err(|_| unavailable("cannot publish updated Note"))?;
        Ok(receipt("updated"))
    }

    pub(crate) fn rename(&self, project_id: Uuid, note_id: Uuid, expected: &str, title: &str) -> Result<Value> {
        let _lock = self.lock_for_write(project_id)?;
        let note = self.find(note_id)?;
        if note.markdown != expected {
            return Err(Failure("conflict", "Note baseline no longer matches"));
        }
        let receipt = |status| json!({"status":status,"project_id":project_id,"note_id":note_id,"title":title});
        if title == note.metadata.title {
            return Ok(receipt("unchanged"));
        }
        let filename = named_filename(title)?;
        let source = note.path.file_name().and_then(|name| name.to_str())
            .ok_or_else(|| Failure("invalid", "Note filename is not UTF-8"))?;
        if !note_filename(source) {
            return Err(Failure("invalid", "Note source filename cannot be retained safely"));
        }
        let destination = self.notes.join(&filename);
        let same_path = source == filename;
        let mut collision = false;
        let folded = title.to_lowercase();
        let entries = self.notes(|other| {
            collision |= other.metadata.brain_id != note_id && other.metadata.title.to_lowercase() == folded;
        })?;
        if collision || (!same_path && !absent(&destination)?) {
            return Err(Failure("conflict", "Note title or filename already exists"));
        }
        // Two stage files and then a retained intent plus possibly two .md files.
        if entries + 2 > MAX_ENTRIES {
            return Err(unavailable("Notes directory has insufficient rename capacity"));
        }
        let after = revised_title(&note, title)?;
        let permissions = fs::symlink_metadata(&note.path)
            .map_err(|_| unavailable("cannot inspect Note permissions"))?.permissions();
        let staged = self.stage(&after, Some(permissions))?;
        let intent = RenameIntent { version: 1, project_id, note_id, source: source.into(), destination: filename, before: note.markdown.clone(), after };
        let encoded = serde_json::to_string(&intent).map_err(|_| unavailable("cannot encode rename intent"))?;
        let pending = self.stage_bytes(&encoded, None, MAX_INTENT)?;
        self.recheck()?;
        if !matches!(read_regular(&note.path, MAX_DOCUMENT), Ok(current) if current == intent.before)
            || (!same_path && !absent(&destination)?) {
            return Err(Failure("conflict", "Note paths changed before publication"));
        }
        let intent_path = self.notes.join(RENAME_INTENT);
        pending.persist_noclobber(&intent_path).map_err(|_| rename_partial())?;
        let verify = |path: &Path, expected: &str| -> Result<()> {
            if matches!(read_regular(path, MAX_DOCUMENT), Ok(current) if current == expected) { Ok(()) } else { Err(rename_partial()) }
        };
        let verify_intent = || {
            if matches!(read_regular(&intent_path, MAX_INTENT), Ok(current) if current == encoded) {
                Ok(())
            } else {
                Err(rename_partial())
            }
        };
        verify_intent()?;
        verify(&note.path, &intent.before)?;
        if same_path {
            // Only the exact baseline file itself is replaced, as in update.
            staged.persist(&destination).map_err(|_| rename_partial())?;
        } else {
            staged.persist_noclobber(&destination).map_err(|_| rename_partial())?;
            verify(&destination, &intent.after)?;
            verify_intent()?;
            self.recheck().map_err(|_| rename_partial())?;
            verify(&note.path, &intent.before)?;
            fs::remove_file(&note.path).map_err(|_| rename_partial())?;
            if !absent(&note.path).map_err(|_| rename_partial())? { return Err(rename_partial()); }
        }
        verify(&destination, &intent.after)?;
        verify_intent()?;
        fs::remove_file(intent_path).map_err(|_| rename_partial())?;
        Ok(receipt("renamed"))
    }

    pub(crate) fn inspect_rename(&self, project_id: Uuid) -> Result<Value> {
        if project_id != self.binding.project_id {
            return Err(Failure("conflict", "active Project changed"));
        }
        let path = self.notes.join(RENAME_INTENT);
        if absent(&path)? { return Ok(json!({"status":"no_intent","project_id":project_id})); }
        let _lock = self.lock_for_read(&self.notes.join(LOCK)).map_err(|error| {
            if error.0 == "busy" { error } else { rename_partial() }
        })?;
        self.recheck()?;
        if absent(&path)? { return Ok(json!({"status":"no_intent","project_id":project_id})); }
        let bytes = read_regular(&path, MAX_INTENT).map_err(|_| rename_partial())?;
        let intent: RenameIntent = serde_json::from_str(&bytes).map_err(|_| rename_partial())?;
        if intent.version != 1 || intent.project_id != project_id || intent.note_id.is_nil()
            || !note_filename(&intent.source) || !note_filename(&intent.destination) {
            return Err(rename_partial());
        }
        let observed = |name: &str| match absent(&self.notes.join(name)) {
            Ok(true) => "absent",
            Ok(false) => match read_regular(&self.notes.join(name), MAX_DOCUMENT) {
                Ok(now) if now == intent.before => "before",
                Ok(now) if now == intent.after => "after",
                Ok(_) => "different",
                Err(_) => "unavailable",
            },
            Err(_) => "unavailable",
        };
        Ok(json!({"status":"partial","project_id":project_id,"note_id":intent.note_id,
            "source":intent.source,"destination":intent.destination,
            "source_state":observed(&intent.source),"destination_state":observed(&intent.destination)}))
    }

    fn stage(
        &self,
        markdown: &str,
        permissions: Option<fs::Permissions>,
    ) -> Result<tempfile::NamedTempFile> {
        self.stage_bytes(markdown, permissions, MAX_DOCUMENT)
    }

    fn stage_bytes(&self, content: &str, permissions: Option<fs::Permissions>, limit: usize) -> Result<tempfile::NamedTempFile> {
        if content.len() > limit {
            return Err(Failure("invalid", "Note publication exceeds byte limit"));
        }
        // Stage within `notas/` so publication remains on the same filesystem.
        // The staging entry is never Markdown and is not exposed by the protocol.
        let mut file = tempfile::NamedTempFile::new_in(&self.notes)
            .map_err(|_| unavailable("cannot stage Note"))?;
        let staged = (|| -> io::Result<()> {
            file.write_all(content.as_bytes())?;
            if let Some(permissions) = permissions {
                file.as_file().set_permissions(permissions)?;
            }
            file.as_file().sync_all()
        })();
        staged.map_err(|_| unavailable("cannot synchronize staged Note"))?;
        Ok(file)
    }
}
