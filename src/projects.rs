//! Projects is an explicit, trusted-host workflow, not an autonomous vault scan.
//! Only 01-proyectos is enumerated. IDs and previews never grant permission.
use crate::{document::{CommonMetadata, DocumentMetadata}, memory::{markdown::ProjectAssociation, store::{Context, Failure, Result, Store}}, project_binding::{Marker, Principal}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, fs::{self, File, OpenOptions}, io::{self, Write}, path::{Path, PathBuf}};
use uuid::Uuid;

const MAX_ENTRIES: usize = 256;
const MAX_DOCUMENT: usize = 512 * 1024;
const MAX_MARKER: usize = 8192;
const HISTORY_BYTES: usize = 32768;
const PROJECT_LOCK: &str = ".brain-projects.lock";

fn fail(status: &str, message: &str) -> Failure {
    Failure { status: status.into(), message: message.into(), memory_id: None, io_kind: None, reason: None }
}
fn io_failure(_: impl std::fmt::Display) -> Failure { fail("unavailable", "Project filesystem or configuration unavailable") }
fn invalid(_: impl std::fmt::Display) -> Failure { fail("invalid", "invalid Project document or binding") }
fn convert(error: crate::notes::Failure) -> Failure { fail(error.0, error.1) }
fn regular(path: &Path, limit: usize) -> Result<String> { crate::notes::read_regular(path, limit).map_err(convert) }
fn optional(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_failure(e)),
        Ok(_) => regular(path, MAX_MARKER).map(Some),
    }
}
fn directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) { Ok(()) }
    else { Err(fail("unavailable", "Project directory must be real and existing")) }
}
pub(crate) fn vault() -> Result<PathBuf> {
    let config = crate::config::load_config().map_err(io_failure)?;
    directory(&config.vault_path)?;
    config.vault_path.canonicalize().map_err(io_failure)
}
fn cwd() -> Result<PathBuf> { std::env::current_dir().map_err(io_failure)?.canonicalize().map_err(io_failure) }
fn context(cwd: &Path) -> Context { Context { working_directory: cwd.to_path_buf(), authorized_boundary: cwd.to_path_buf() } }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Project {
    pub project_id: Uuid,
    pub title: String,
    pub path: String,
}
impl Project {
    fn association(&self) -> ProjectAssociation { ProjectAssociation { brain_project: self.project_id, path: self.path.clone(), label: self.title.clone() } }
    fn principal(&self) -> Principal { Principal { project_id: self.project_id, project_path: self.path.clone(), allow_write: true } }
}

fn active(raw: &str) -> Result<bool> {
    #[derive(Deserialize)]
    struct Header { project_status: Option<String> }
    let tail = raw.strip_prefix("---\r\n").or_else(|| raw.strip_prefix("---\n")).ok_or_else(|| invalid("header"))?;
    let mut offset = 0;
    for line in tail.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let header: Header = serde_saphyr::from_str(&tail[..offset]).map_err(invalid)?;
            return match header.project_status.as_deref() {
                None | Some("active") => Ok(true),
                Some("archived") => Ok(false),
                _ => Err(invalid("unknown project_status")),
            };
        }
        offset += line.len();
    }
    Err(invalid("header"))
}

/// Compatibility: existing Project v1 documents without project_status are active.
/// An ambiguous identity or malformed candidate fails the whole listing closed.
pub(crate) fn catalog(vault: &Path) -> Result<Vec<Project>> {
    let area = vault.join("01-proyectos");
    directory(&area)?;
    let mut projects = Vec::new();
    let mut identities = HashSet::new();
    let mut paths = Vec::new();
    let mut entries = 0;
    for entry in fs::read_dir(&area).map_err(io_failure)? {
        let entry = entry.map_err(io_failure)?;
        if entry.file_name() == PROJECT_LOCK { continue; }
        entries += 1;
        if entries > MAX_ENTRIES { return Err(fail("too_large", "Project area exceeds entry limit")); }
        let kind = entry.file_type().map_err(io_failure)?;
        if kind.is_symlink() { return Err(fail("unavailable", "Project area contains a symlink")); }
        if kind.is_dir() { paths.push(entry.path()); }
    }
    paths.sort();
    for dir in paths {
        let folder = dir.file_name().and_then(|s| s.to_str()).ok_or_else(|| invalid("folder"))?;
        if !crate::project_binding::safe_component(folder) { return Err(invalid("unsafe folder")); }
        let mut count = 0;
        for path in crate::notes::direct_markdown(&dir).map_err(convert)?.0 {
            let raw = regular(&path, MAX_DOCUMENT)?;
            let Some((metadata, _)) = crate::notes::metadata(&raw, "project").map_err(convert)? else { continue; };
            count += 1;
            if count != 1 || !identities.insert(metadata.common().brain_id) {
                return Err(fail("conflict", "Project identity or folder is ambiguous"));
            }
            let name = path.file_name().and_then(|s| s.to_str()).ok_or_else(|| invalid("filename"))?;
            let path = format!("01-proyectos/{folder}/{name}");
            if !crate::project_binding::valid_project_path(&path) { return Err(invalid("path")); }
            let title = &metadata.common().title;
            if !valid_title(title) { return Err(invalid("title")); }
            if active(&raw)? {
                projects.push(Project { project_id: metadata.common().brain_id, title: title.clone(), path });
            }
        }
    }
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(projects)
}
pub(crate) fn find(vault: &Path, id: Uuid) -> Result<Project> {
    if id.is_nil() { return Err(invalid("nil identity")); }
    catalog(vault)?.into_iter().find(|p| p.project_id == id).ok_or_else(|| fail("not_found", "active Project not found"))
}
pub(crate) fn association(vault: &Path, id: Uuid) -> Result<ProjectAssociation> { Ok(find(vault, id)?.association()) }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Legacy { binding_version: u32, project_id: Uuid, project_path: String, #[serde(default)] allow_write: bool }

fn principal_from(marker: Option<&str>, legacy: Option<&str>) -> Result<(Option<Principal>, &'static str)> {
    if let Some(raw) = marker {
        let marker = Marker::parse(raw).map_err(invalid)?;
        if marker.binding_version == 2 {
            return Ok((marker.principal_project, "unified"));
        }
    }
    if let Some(raw) = legacy {
        let old: Legacy = toml::from_str(raw).map_err(invalid)?;
        if old.binding_version != 1 || old.project_id.is_nil() || !crate::project_binding::valid_project_path(&old.project_path) {
            return Err(invalid("legacy binding"));
        }
        return Ok((Some(Principal { project_id: old.project_id, project_path: old.project_path, allow_write: old.allow_write }), "legacy"));
    }
    Ok((None, "none"))
}

/// Used by Notes. A v2 marker always takes precedence; malformed v2 never falls
/// back. Explicit additional scopes are constructed elsewhere as read-only.
pub(crate) fn notes_principal() -> Result<Option<Principal>> {
    let cwd = cwd()?;
    let marker = optional(&cwd.join(".brain.toml"))?;
    if let Some(raw) = marker.as_deref() {
        let parsed = Marker::parse(raw).map_err(invalid)?;
        if parsed.binding_version == 2 { return Ok(parsed.principal_project); }
    }
    principal_from(marker.as_deref(), optional(&cwd.join(".brain-notes.toml"))?.as_deref()).map(|(p, _)| p)
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditScope {
    vault_directory: PathBuf,
    marker: Option<String>,
    legacy_marker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Baseline {
    vault_directory: PathBuf,
    marker: Option<String>,
    legacy_marker: Option<String>,
    memory: Value,
}
fn marker_bytes(cwd: &Path) -> Result<(Option<String>, Option<String>)> {
    let marker = optional(&cwd.join(".brain.toml"))?;
    let unified = marker.as_deref().map(Marker::parse).transpose().map_err(invalid)?
        .is_some_and(|m| m.binding_version == 2);
    let legacy_marker = if unified { None } else { optional(&cwd.join(".brain-notes.toml"))? };
    Ok((marker, legacy_marker))
}
fn baseline(cwd: &Path, store: &Store) -> Result<Baseline> {
    let (marker, legacy_marker) = marker_bytes(cwd)?;
    principal_from(marker.as_deref(), legacy_marker.as_deref())?;
    let memory = store.resolve(&context(cwd))?;
    Ok(Baseline { vault_directory: store.vault_directory().to_path_buf(), marker, legacy_marker, memory })
}
fn validate_baseline(cwd: &Path, store: &Store, expected: &Baseline) -> Result<()> {
    if vault()? != store.vault_directory() || baseline(cwd, store)? != *expected { return Err(fail("conflict", "Project or Memory baseline changed")); }
    match expected.memory["status"].as_str() {
        Some("found" | "unbound") => Ok(()),
        Some("manual_review") => Err(fail("manual_review", "external Memory edit requires review")),
        _ => Err(fail("partial", "Memory baseline is incomplete; no binding writes")),
    }
}

fn valid_title(title: &str) -> bool {
    !title.trim().is_empty() && title.len() <= 256 && !title.chars().any(char::is_control)
        && !title.contains(['[', ']', '|'])
}
/// Creation-only policy: never sanitize the filename and never change the
/// compatibility rules used by catalog or existing bindings.
fn validate_creation_title(title: &str) -> Result<()> {
    let invalid_filename = |message: &str| {
        let mut error = fail("invalid", message);
        error.reason = Some("INVALID_PROJECT_FILENAME".into());
        error
    };
    let filename = format!("{title}.md");
    if filename.len() > 255 || filename.encode_utf16().count() > 255 {
        return Err(invalid_filename("Project filename including .md must fit 255 UTF-8 bytes and 255 UTF-16 units; edit the title"));
    }
    if !valid_title(title) || title.trim() != title || title.ends_with('.')
        || !crate::project_binding::safe_component(&filename) {
        return Err(invalid_filename("Project title must be a portable filename without separators, controls, forbidden delimiters, edge whitespace or trailing dots; edit the title"));
    }
    if crate::project_binding::reserved_windows_stem(title) {
        return Err(invalid_filename("Project title is a reserved Windows device filename, including extension variants; edit the title"));
    }
    Ok(())
}

fn valid_body(body: &str) -> bool {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag};
    let names = ["Summary", "Objective", "Scope", "References"];
    let mut level = 0usize;
    let mut sections = 0;
    for (event, range) in Parser::new(body).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level: HeadingLevel::H2, .. }) => {
                if level != 0 || sections >= 4 { return false; }
                let line = body[range].lines().next().unwrap_or("").trim_end_matches('\r');
                if line != format!("## {}", names[sections]) { return false; }
                sections += 1;
                level += 1;
            }
            Event::Start(_) => level += 1,
            Event::End(_) => level = level.saturating_sub(1),
            _ => {}
        }
    }
    (3..=4).contains(&sections) && body.len() <= MAX_DOCUMENT
}
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() { slug.push(c.to_ascii_lowercase()); }
        else if !slug.ends_with('-') { slug.push('-'); }
        if slug.len() >= 48 { break; }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() { "proyecto".into() } else { slug.into() }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preview { project_id: Uuid, title: String, body: String, path: String }
impl Preview {
    pub(crate) fn project_id(&self) -> Uuid { self.project_id }
    fn path(id: Uuid, title: &str) -> String { format!("01-proyectos/{}-{id}/{title}.md", slug(title)) }
    fn validate(&self) -> Result<()> {
        validate_creation_title(&self.title)?;
        if self.project_id.is_nil() || !valid_body(&self.body)
            || self.path != Self::path(self.project_id, &self.title) {
            return Err(invalid("preview"));
        }
        Ok(())
    }
    fn project(&self) -> Project { Project { project_id: self.project_id, title: self.title.clone(), path: self.path.clone() } }
    fn markdown(&self) -> Result<String> {
        self.validate()?;
        let metadata = DocumentMetadata::Project(CommonMetadata { brain_version: 1, brain_id: self.project_id, title: self.title.clone() });
        let yaml = serde_saphyr::to_string(&metadata).map_err(invalid)?;
        let raw = format!("---\n{}---\n{}", yaml, self.body);
        if raw.len() > MAX_DOCUMENT { return Err(fail("too_large", "Project exceeds document bound")); }
        let decoded = crate::notes::metadata(&raw, "project").map_err(convert)?;
        if decoded.as_ref().map(|(m, _)| m) != Some(&metadata) { return Err(invalid("roundtrip")); }
        Ok(raw)
    }
}

pub(crate) fn status() -> Result<Value> {
    let cwd = cwd()?;
    let vault = vault()?;
    let store = Store::open(&vault)?;
    let expected = baseline(&cwd, &store)?;
    let (principal, binding) = principal_from(expected.marker.as_deref(), expected.legacy_marker.as_deref())?;
    let allow_write = principal.as_ref().is_some_and(|p| p.allow_write);
    let principal = principal.map(|p| {
        let project = find(&vault, p.project_id)?;
        if project.path != p.project_path { return Err(fail("conflict", "bound Project identity/path changed")); }
        Ok(project)
    }).transpose()?;
    Ok(json!({"status":if principal.is_some() {"found"} else {"unbound"},"binding":binding,
        "principal":principal,"allow_write":allow_write,"expected":expected}))
}
pub(crate) fn list() -> Result<Value> { Ok(json!({"status":"listed","projects":catalog(&vault()?)?})) }
pub(crate) fn read(id: Uuid) -> Result<Value> {
    let vault = vault()?;
    let project = find(&vault, id)?;
    let markdown = regular(&vault.join(&project.path), MAX_DOCUMENT)?;
    let (metadata, start) = crate::notes::metadata(&markdown, "project").map_err(convert)?.ok_or_else(|| invalid("Project"))?;
    if metadata.common().brain_id != id || metadata.common().title != project.title || !active(&markdown)? {
        return Err(fail("conflict", "Project changed during read"));
    }
    Ok(json!({"status":"found","project":project,"body":&markdown[start..],"markdown":markdown}))
}
/// Change only the body of the exact writable Project already bound to this cwd.
/// No Memory association, note directory creation or marker publication.
pub(crate) fn update_body(project: Project, expected: EditScope, expected_markdown: &str, body: &str, approved: bool) -> Result<Value> {
    if !approved { return Err(fail("denied", "explicit host human confirmation required")); }
    if expected_markdown.len() > MAX_DOCUMENT || !valid_body(body) || body.trim_start().lines().next() == Some("---") {
        return Err(invalid("Project body"));
    }
    let cwd = cwd()?;
    let vault_dir = vault()?;
    let check_scope = || {
        let (marker, legacy_marker) = marker_bytes(&cwd)?;
        if vault_dir != expected.vault_directory || vault()? != vault_dir || marker != expected.marker || legacy_marker != expected.legacy_marker {
            return Err(fail("conflict", "Project edit scope changed"));
        }
        let (principal, _) = principal_from(marker.as_deref(), legacy_marker.as_deref())?;
        let principal = principal.ok_or_else(|| fail("denied", "Project editing requires a writable principal"))?;
        if !principal.allow_write || principal.project_id != project.project_id || principal.project_path != project.path {
            return Err(fail("denied", "Project editing requires the exact writable principal"));
        }
        Ok(())
    };
    let check = || {
        check_scope()?;
        let actual = find(&vault_dir, project.project_id).map_err(|e| if e.status == "not_found" {
            fail("conflict", "confirmed Project is no longer active")
        } else { e })?;
        if actual != project || regular(&vault_dir.join(&actual.path), MAX_DOCUMENT)? != expected_markdown {
            return Err(fail("conflict", "confirmed Project document changed"));
        }
        Ok(())
    };
    check()?;
    let _binding_lock = binding_lock(&cwd)?;
    let _project_lock = acquire_lock(&vault_dir.join("01-proyectos").join(PROJECT_LOCK))?;
    check()?;
    let (_, start) = crate::notes::metadata(expected_markdown, "project").map_err(convert)?.ok_or_else(|| invalid("Project"))?;
    let receipt = |status| json!({"status":status,"project":project});
    if &expected_markdown[start..] == body { return Ok(receipt("unchanged")); }
    let mut markdown = expected_markdown[..start].to_owned();
    // A legacy empty file may end at the closing delimiter. Keep its existing bytes.
    if !markdown.ends_with('\n') { markdown.push_str(if markdown.starts_with("---\r\n") { "\r\n" } else { "\n" }); }
    markdown.push_str(body);
    if markdown.len() > MAX_DOCUMENT { return Err(fail("invalid", "Project exceeds document bound")); }
    let file = vault_dir.join(&project.path);
    let parent = file.parent().expect("validated Project path");
    let permissions = fs::symlink_metadata(&file).map_err(io_failure)?.permissions();
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(io_failure)?;
    staged.write_all(markdown.as_bytes()).map_err(io_failure)?;
    staged.as_file().set_permissions(permissions).map_err(io_failure)?;
    staged.as_file().sync_all().map_err(io_failure)?;
    // Do not enumerate the Project folder with our temporary file present.
    check_scope()?;
    directory(&vault_dir.join("01-proyectos"))?;
    directory(parent)?;
    if !matches!(regular(&file, MAX_DOCUMENT), Ok(current) if current == expected_markdown) {
        return Err(fail("conflict", "Project changed while staging"));
    }
    // Same-filesystem file replacement, serialized against cooperating Brain writers.
    if let Err(e) = staged.persist(&file) {
        if matches!(regular(&file, MAX_DOCUMENT), Ok(current) if current == expected_markdown) {
            return Err(io_failure(e));
        }
        return Err(fail("partial", "Project replacement outcome is uncertain; inspect without retrying"));
    }
    if check_scope().is_err() || !matches!(regular(&file, MAX_DOCUMENT), Ok(current) if current == markdown) {
        return Err(fail("partial", "Project replacement could not be verified; inspect without retrying"));
    }
    Ok(receipt("updated"))
}

pub(crate) fn preview_create(title: String, body: String) -> Result<Value> {
    let project_id = Uuid::new_v4();
    let path = Preview::path(project_id, &title);
    let preview = Preview { project_id, title, body, path };
    preview.markdown()?;
    let cwd = cwd()?;
    let vault = vault()?;
    let store = Store::open(&vault)?;
    let expected = baseline(&cwd, &store)?;
    validate_baseline(&cwd, &store, &expected)?;
    store.with_project_guard(&context(&cwd), &expected.memory, |guard| guard.prepare(&preview.project().association()))?;
    project_capacity(&vault)?;
    if catalog(&vault)?.iter().any(|p| p.project_id == project_id) || fs::symlink_metadata(vault.join(&preview.path).parent().expect("owned path")).is_ok() {
        return Err(fail("conflict", "preview destination already exists"));
    }
    Ok(json!({"status":"preview","preview":preview,"expected":expected}))
}
pub(crate) fn draft_context() -> Result<Value> {
    let cwd = cwd()?;
    let store = Store::open(&vault()?)?;
    let memory = store.resolve(&context(&cwd))?;
    let history = store.selected_history(&context(&cwd), &memory, HISTORY_BYTES)
        .unwrap_or_else(|e| json!({"status":"unavailable","reason":e.reason.unwrap_or(e.status)}));
    Ok(json!({"status":"found","memory":memory,"history":history}))
}

fn project_capacity(vault: &Path) -> Result<()> {
    let mut count = 0;
    for entry in fs::read_dir(vault.join("01-proyectos")).map_err(io_failure)? {
        if entry.map_err(io_failure)?.file_name() != PROJECT_LOCK { count += 1; }
    }
    if count >= MAX_ENTRIES { return Err(fail("too_large", "no capacity for another Project directory")); }
    Ok(())
}

struct BindingLock(File);
impl Drop for BindingLock { fn drop(&mut self) { let _ = self.0.unlock(); } }
fn binding_lock(cwd: &Path) -> Result<BindingLock> {
    acquire_lock(&cwd.join(".brain.toml.lock"))
}
fn acquire_lock(path: &Path) -> Result<BindingLock> {
    match fs::symlink_metadata(&path) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err(fail("unavailable", "invalid binding lock")),
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_failure(e)),
        _ => {}
    }
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(io_failure)?;
    file.try_lock().map_err(|e| match e {
        fs::TryLockError::WouldBlock => fail("busy", "binding lock is held"),
        fs::TryLockError::Error(e) => io_failure(e),
    })?;
    Ok(BindingLock(file))
}
fn publish_marker(cwd: &Path, expected: &Baseline, next: &str) -> Result<()> {
    let unified = expected.marker.as_deref().map(Marker::parse).transpose().map_err(invalid)?
        .is_some_and(|m| m.binding_version == 2);
    if optional(&cwd.join(".brain.toml"))? != expected.marker
        || (!unified && optional(&cwd.join(".brain-notes.toml"))? != expected.legacy_marker) {
        return Err(fail("conflict", "binding changed before publication"));
    }
    let mut staged = tempfile::NamedTempFile::new_in(cwd).map_err(io_failure)?;
    staged.write_all(next.as_bytes()).map_err(io_failure)?;
    staged.as_file().sync_all().map_err(io_failure)?;
    if optional(&cwd.join(".brain.toml"))? != expected.marker { return Err(fail("conflict", "binding changed while staging")); }
    if expected.marker.is_some() { staged.persist(cwd.join(".brain.toml")).map_err(io_failure)?; }
    else { staged.persist_noclobber(cwd.join(".brain.toml")).map_err(io_failure)?; }
    if optional(&cwd.join(".brain.toml"))?.as_deref() != Some(next) { return Err(fail("partial", "binding verification failed")); }
    Ok(())
}

/// One host confirmation covers creation (optional), Memory association and
/// marker publication. This is NOT a multi-file transaction. After any write,
/// uncertainty is returned explicitly and never causes allocation/retry.
pub(crate) fn bind(request: Uuid, id: Uuid, preview: Option<Preview>, expected_project: Option<Project>, expected: Baseline, approved: bool) -> Result<Value> {
    if !approved { return Err(fail("denied", "explicit host human confirmation required")); }
    if preview.is_some() == expected_project.is_some() {
        return Err(invalid("exactly one creation preview or expected existing Project is required"));
    }
    if expected_project.as_ref().is_some_and(|p| p.project_id != id) {
        return Err(fail("conflict", "confirmed Project identity differs from selected identity"));
    }
    let cwd = cwd()?;
    let vault = vault()?;
    let store = Store::open(&vault)?;
    validate_baseline(&cwd, &store, &expected)?;
    let project = if let Some(preview) = &preview {
        preview.validate()?;
        if id != preview.project_id { return Err(invalid("preview identity")); }
        preview.project()
    } else {
        let selected = find(&vault, id).map_err(|e| if e.status == "not_found" {
            fail("conflict", "confirmed Project no longer exists as an active Project")
        } else { e })?;
        if expected_project.as_ref() != Some(&selected) {
            return Err(fail("conflict", "confirmed Project identity, title or path changed"));
        }
        selected
    };
    // Reject an already divergent confirmation before even initializing locks.
    let _lock = binding_lock(&cwd)?;
    validate_baseline(&cwd, &store, &expected)?;
    let next = crate::project_binding::bind_bytes(expected.marker.as_deref(), project.principal()).map_err(invalid)?;
    if next.len() > MAX_MARKER { return Err(fail("too_large", "unified marker exceeds byte bound")); }
    let association = project.association();
    store.with_project_guard(&context(&cwd), &expected.memory, |guard| {
        guard.prepare(&association)?;
        let _project_lock = acquire_lock(&vault.join("01-proyectos").join(PROJECT_LOCK))?;
        let mut created = false;
        let mut reserved = false;
        let mut notes_created = false;
        let result = (|| {
            if let Some(preview) = &preview {
                let markdown = preview.markdown()?;
                project_capacity(&vault)?;
                if catalog(&vault)?.iter().any(|p| p.project_id == id) {
                    return Err(fail("conflict", "preview identity already exists; select existing Project"));
                }
                let file = vault.join(&preview.path);
                let parent = file.parent().expect("owned preview path");
                // Creating the unique directory is the allocation reservation.
                // Retain it after any later error, never choose a fresh path.
                fs::create_dir(parent).map_err(|e| if e.kind() == io::ErrorKind::AlreadyExists {
                    fail("conflict", "preview directory already exists; inspect, do not recreate")
                } else { io_failure(e) })?;
                reserved = true;
                fs::create_dir(parent.join("notas")).map_err(io_failure)?;
                let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(io_failure)?;
                staged.write_all(markdown.as_bytes()).map_err(io_failure)?;
                staged.as_file().sync_all().map_err(io_failure)?;
                staged.persist_noclobber(&file).map_err(io_failure)?;
                created = true;
            }
            let selected = find(&vault, id)?;
            if selected != project || expected_project.as_ref().is_some_and(|p| *p != selected) {
                return Err(fail("conflict", "confirmed Project changed before binding"));
            }
            let notes = vault.join(&project.path).parent().expect("validated Project path").join("notas");
            match fs::symlink_metadata(&notes) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&notes).map_err(io_failure)?;
                    notes_created = true;
                }
                Err(e) => return Err(io_failure(e)),
                Ok(_) => directory(&notes)?,
            }
            let memory = guard.associate(request, &association)?;
            publish_marker(&cwd, &expected, &next).map_err(|mut e| {
                e.status = "partial".into();
                e.reason = Some("memory_may_be_associated_but_principal_not_published".into());
                e
            })?;
            Ok(json!({"status":"bound","project":project,"allow_write":true,"memory":memory,"created":created,"notes_created":notes_created}))
        })();
        match result {
            Ok(receipt) => Ok(receipt),
            Err(error) if reserved || notes_created || error.status == "partial" => Ok(json!({"status":"partial",
                "project_created":created,"project_reserved":reserved,"notes_created":notes_created,"project":project,"reason":error.reason.unwrap_or(error.status),
                "message":"operation incomplete; inspect retained Project and Memory state; never recreate automatically"})),
            Err(error) => Err(error),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_body_requires_ordered_real_top_level_sections() {
        let valid = "## Summary\nResumen\n## Objective\nObjetivo\n## Scope\nAlcance\n";
        assert!(valid_body(valid));
        assert!(valid_body(&format!("{valid}## References\nAntecedentes\n")));
        assert!(!valid_body("## Objective\n## Summary\n## Scope\n"));
        assert!(!valid_body("```\n## Summary\n## Objective\n## Scope\n```"));
        assert!(!valid_body(&valid.replace("## Objective", "> ## Objective")));
    }
    #[test]
    fn preview_owns_safe_destination_and_rejects_tampering() {
        let id = Uuid::from_u128(1);
        let title = "Proyecto Español Interior".to_owned();
        let mut preview = Preview { project_id: id, path: Preview::path(id, &title), title,
            body: "## Summary\nResumen\n## Objective\nObjetivo\n## Scope\nAlcance\n".into() };
        assert!(preview.markdown().is_ok());
        for filename in ["Proyecto.md", "proyecto-espa-ol-interior.md", "Otro.md"] {
            preview.path = format!("01-proyectos/{}-{id}/{filename}", slug(&preview.title));
            assert!(preview.validate().is_err());
        }
        preview.path = "01-proyectos/../outside.md".into();
        assert!(preview.validate().is_err());
    }
    #[test]
    fn creation_filename_preserves_portable_titles_and_rejects_instead_of_sanitizing() {
        for title in ["StelaShell", "Análisis de Datos", "Árbol  Español", "COM0", "COM10", "LPT10", "CONSOLE"] {
            assert!(validate_creation_title(title).is_ok(), "{title}");
            let path = Preview::path(Uuid::from_u128(1), title);
            assert_eq!(path.rsplit('/').next().unwrap(), format!("{title}.md"));
        }
        for title in ["", " ", "Nombre/Parte", "Nombre\\Parte", "Nombre:Parte", "Nombre?", "Nombre#", "[Nombre]",
            " Nombre", "Nombre ", "Nombre.", ".", "..", "Nombre\n", "CON", "con", "NUL.txt", "PRN", "AUX",
            "COM1", "LPT9", "COM¹", "com².txt", "LPT³", "CON .txt", "CONIN$", "conout$.log", "CLOCK$"] {
            let error = validate_creation_title(title).unwrap_err();
            assert_eq!(error.status, "invalid", "{title}");
            assert_eq!(error.reason.as_deref(), Some("INVALID_PROJECT_FILENAME"));
        }
        assert!(validate_creation_title(&"a".repeat(252)).is_ok());
        assert!(validate_creation_title(&"a".repeat(253)).is_err());
        assert!(validate_creation_title(&"á".repeat(126)).is_ok());
        assert!(validate_creation_title(&format!("{}a", "á".repeat(126))).is_err());
    }
}
