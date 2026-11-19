//! The host owns cwd and Brain's existing vault configuration. This endpoint
//! resolves exactly one Project directory and never consults Memory bindings.
use crate::document::{CommonMetadata, DOCUMENT_VERSION, DocumentMetadata};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const MARKER: &str = ".brain-notes.toml";
const LOCK: &str = ".brain-notes.lock";
const MAX_ENTRIES: usize = 256;
const MAX_DOCUMENT: usize = 512 * 1024;
const MAX_MARKER: usize = 8 * 1024;
const MAX_TITLE: usize = 256;

#[derive(Debug)]
pub(crate) struct Failure(pub(crate) &'static str, pub(crate) &'static str);
type Result<T> = std::result::Result<T, Failure>;

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

fn read_regular(path: &Path, limit: usize) -> Result<String> {
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
fn metadata(markdown: &str, kind: &str) -> Result<Option<(DocumentMetadata, usize)>> {
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

fn direct_markdown(dir: &Path) -> Result<(Vec<PathBuf>, usize)> {
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
        let cwd =
            std::env::current_dir().map_err(|_| unavailable("cannot resolve host context"))?;
        let marker = cwd.join(MARKER);
        match fs::symlink_metadata(&marker) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Failure(
                    "unbound",
                    "no Notes binding in the host working directory",
                ));
            }
            Err(_) => return Err(unavailable("cannot inspect Notes binding")),
            Ok(_) => {}
        }
        let input = read_regular(&marker, MAX_MARKER)?;
        let binding: Binding =
            toml::from_str(&input).map_err(|_| unavailable("invalid Notes binding"))?;
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
        Ok(entries)
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

    pub(crate) fn list(&self) -> Result<Value> {
        let mut notes = Vec::new();
        self.notes(|note| {
            notes.push(json!({"note_id": note.metadata.brain_id, "title": note.metadata.title}))
        })?;
        Ok(json!({"status": "listed", "project_id": self.binding.project_id, "notes": notes}))
    }

    pub(crate) fn read(&self, note_id: Uuid) -> Result<Value> {
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
        if !valid_title(&title) {
            return Err(Failure(
                "invalid",
                "title must be nonblank, at most 256 UTF-8 bytes and without control characters",
            ));
        }
        let _lock = self.lock_for_write(project_id)?;
        // Avoid making a 256-entry directory unreadable by creating entry 257.
        let note_id = Uuid::new_v4();
        let mut occupied = false;
        if self.notes(|note| occupied |= note.metadata.brain_id == note_id)? >= MAX_ENTRIES {
            return Err(unavailable("Notes directory has reached the entry limit"));
        }
        if occupied {
            return Err(Failure(
                "conflict",
                "generated Note identity is already used",
            ));
        }
        let metadata = DocumentMetadata::Note(CommonMetadata {
            brain_version: DOCUMENT_VERSION,
            brain_id: note_id,
            title: title.clone(),
        });
        let yaml = serde_saphyr::to_string(&metadata)
            .map_err(|_| Failure("invalid", "cannot encode Note metadata"))?;
        let markdown = format!("---\n{}\n---\n{body}", yaml.trim_end_matches('\n'));
        // Do not publish a title that the Markdown codec cannot round-trip.
        if !matches!(self::metadata(&markdown, "note"), Ok(Some((decoded, _))) if decoded == metadata)
        {
            return Err(Failure("invalid", "cannot round-trip Note metadata"));
        }
        let staged = self.stage(&markdown, None)?;
        self.recheck()?;
        staged
            .persist_noclobber(self.notes.join(format!("{note_id}.md")))
            .map_err(|e| {
                if e.error.kind() == io::ErrorKind::AlreadyExists {
                    Failure("conflict", "generated Note file already exists")
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

    fn stage(
        &self,
        markdown: &str,
        permissions: Option<fs::Permissions>,
    ) -> Result<tempfile::NamedTempFile> {
        if markdown.len() > MAX_DOCUMENT {
            return Err(Failure("invalid", "Note exceeds document byte limit"));
        }
        // Stage within `notas/` so publication remains on the same filesystem.
        // The staging entry is never Markdown and is not exposed by the protocol.
        let mut file = tempfile::NamedTempFile::new_in(&self.notes)
            .map_err(|_| unavailable("cannot stage Note"))?;
        let staged = (|| -> io::Result<()> {
            file.write_all(markdown.as_bytes())?;
            if let Some(permissions) = permissions {
                file.as_file().set_permissions(permissions)?;
            }
            file.as_file().sync_all()
        })();
        staged.map_err(|_| unavailable("cannot synchronize staged Note"))?;
        Ok(file)
    }
}
