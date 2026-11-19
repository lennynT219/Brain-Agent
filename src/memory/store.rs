//! Local cooperating-writer storage. Caller authority is external to IDs/paths.
//! No filesystem CAS, multi-file transaction or power-loss guarantee is made.
use super::{
    CURRENT_SECTIONS,
    binding::{self, Binding, Resolution},
    change_log::{self, HEAD},
    history::{AcceptedSnapshot, History},
    history_link,
    markdown::{self, MemoryDocument, ProjectAssociation},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for Failure {}
fn fail(status: &str, message: &str) -> Failure {
    Failure {
        status: status.into(),
        message: message.into(),
        memory_id: None,
        io_kind: None,
        reason: None,
    }
}
fn io(error: std::io::Error) -> Failure {
    let mut failure = fail("unavailable", "filesystem operation failed");
    failure.io_kind = Some(format!("{:?}", error.kind()));
    failure
}
fn invalid(error: impl Into<anyhow::Error>) -> Failure {
    let error = error.into();
    let mut failure = fail("invalid", "document or arguments failed validation");
    if error
        .downcast_ref::<super::syntax::UnclosedHtml>()
        .is_some()
    {
        failure.reason = Some("UNCLOSED_HTML".into());
    }
    failure
}
fn binding_failure(error: anyhow::Error) -> Failure {
    if let Some(source) = error.downcast_ref::<std::io::Error>() {
        let mut failure = fail("unavailable", "binding or boundary is unavailable");
        failure.io_kind = Some(format!("{:?}", source.kind()));
        failure
    } else {
        fail("invalid", "binding or authorized boundary is invalid")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub working_directory: PathBuf,
    pub authorized_boundary: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub revision: u64,
    pub accepted_markdown: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub context: Context,
    pub title: String,
    pub sections: [String; 5],
    #[serde(default)]
    pub project: Option<ProjectAssociation>,
    #[serde(default)]
    pub memory_subfolders: Vec<String>,
}
/// Explicit host-approved replacement of one genuinely absent reference.
/// Ordinary create never invokes this admission path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceMissing {
    pub context: Context,
    pub binding_directory: PathBuf,
    pub expected_memory_id: Uuid,
    pub expected_marker: String,
    pub human_approved: bool,
    pub title: String,
    pub sections: [String; 5],
    #[serde(default)]
    pub project: Option<ProjectAssociation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub status: String,
    pub memory_id: Uuid,
    pub revision: Option<u64>,
    pub accepted_markdown: Option<String>,
    pub observed_markdown: Option<String>,
    pub approval_candidate: Option<String>,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    pub status: String,
    pub memory_id: Uuid,
    pub revision: u64,
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_memory_id: Option<Uuid>,
}

/// Exact untrusted observation for explicitly approved recovery, not a token
/// that grants permission. Contents may contain sensitive context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub memory_id: Uuid,
    pub current: Option<String>,
    pub history: Option<String>,
    pub pending: Option<String>,
    pub marker: Option<String>,
    pub reservation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reassociation: Option<String>,
    /// Exact bytes only when history cannot be decoded as UTF-8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_bytes: Option<Vec<u8>>,
    #[serde(default)]
    pub history_corrupt: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_head: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    version: u32,
    memory_id: Uuid,
    request_id: Uuid,
    before_current: Option<String>,
    before_history: Option<String>,
    next_current: String,
    next_history: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_head: Option<String>,
    marker: Option<String>,
    #[serde(default)]
    before_marker: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    memory_id: Uuid,
    request_id: Uuid,
    #[serde(default)]
    before_marker: Option<String>,
}
impl Reservation {
    fn parse(raw: &str) -> Result<Self> {
        let r: Self =
            toml::from_str(raw).map_err(|_| fail("partial", "invalid creation reservation"))?;
        if r.version != 1 || r.memory_id.is_nil() || r.request_id.is_nil() {
            return Err(fail("partial", "invalid creation reservation identity"));
        }
        Ok(r)
    }
    fn from_pending(p: &Pending) -> Self {
        Self {
            version: 1,
            memory_id: p.memory_id,
            request_id: p.request_id,
            before_marker: p.before_marker.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Reassociation {
    version: u32,
    request_id: Uuid,
    previous_memory_id: Uuid,
    memory_id: Uuid,
    before_marker: String,
    marker: String,
}
const REASSOCIATION: &str = ".reassociation.json";
const RESERVATION: &str = ".brain.pending.toml";
const PENDING: &str = ".pending.json";

/// Synthetic fault-injection seam. Production callers normally use None.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    AfterReservation,
    AfterPending,
    AfterCurrent,
    AfterHistory,
    AfterHead,
    BeforeMarker,
    AfterMarker,
    AfterReservationCleanup,
}

#[derive(Debug)]
struct Lock(File);
impl Lock {
    fn acquire(path: &Path, create: bool) -> Result<Self> {
        if let Ok(metadata) = fs::symlink_metadata(path)
            && (!metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err(fail("unavailable", "lock is not a regular file"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(path)
            .map_err(io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(std::fs::TryLockError::WouldBlock) => {
                Err(fail("busy", "publication lock is held; no owner inferred"))
            }
            Err(std::fs::TryLockError::Error(error)) => Err(io(error)),
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub struct Store {
    vault: PathBuf,
    fault: Fault,
}

/// Held only during explicit Project binding; never allocates a Memory.
pub(crate) struct ProjectGuard<'a> {
    store: &'a Store,
    baseline: Option<(PathBuf, Lock, Expected, MemoryDocument)>,
}
impl ProjectGuard<'_> {
    pub(crate) fn prepare(&self, project: &ProjectAssociation) -> Result<()> {
        if let Some((_, _, _, document)) = &self.baseline {
            document.with_project(project).map_err(invalid)?;
        }
        Ok(())
    }
    pub(crate) fn associate(&self, request: Uuid, project: &ProjectAssociation) -> Result<serde_json::Value> {
        let Some((dir, _, expected, document)) = &self.baseline else {
            return Ok(serde_json::json!({"status":"deferred"}));
        };
        let id = document.metadata.common().brain_id;
        self.store.check_expected(id, dir, expected, false)?;
        self.store.validate_project(project)?;
        let next = document.with_project(project).map_err(invalid)?;
        if next.raw == document.raw {
            return Ok(serde_json::json!({"status":"unchanged","memory_id":id,"revision":document.revision}));
        }
        let receipt = self.store.successor(dir, request, &document.raw, &document.raw, next,
            Some("Se actualizó la asociación explícita al Project principal."))?;
        Ok(serde_json::json!({"status":"associated","memory_id":id,"revision":receipt.revision}))
    }
}
impl Store {
    pub fn open(vault: &Path) -> Result<Self> {
        crate::vault::validate_vault(vault).map_err(|error| {
            if let Some(source) = error.downcast_ref::<std::io::Error>() {
                let mut f = fail("unavailable", "selected vault is unavailable");
                f.io_kind = Some(format!("{:?}", source.kind()));
                f
            } else {
                fail("unavailable", "selected vault is invalid")
            }
        })?;
        Ok(Self {
            vault: vault.canonicalize().map_err(io)?,
            fault: Fault::None,
        })
    }
    pub(crate) fn vault_directory(&self) -> &Path { &self.vault }

    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = fault;
        self
    }
    fn directory(&self, id: Uuid, create: bool) -> Result<PathBuf> {
        if id.is_nil() {
            return Err(fail("invalid", "nil Memory identity"));
        }
        let area = self.vault.join("05-memorias");
        directory(&area, create)?;
        let dir = area.join(id.to_string());
        directory(&dir, create)?;
        Ok(dir)
    }
    fn lock(&self, id: Uuid, create: bool) -> Result<(PathBuf, Lock)> {
        let dir = self.directory(id, create)?;
        let lock = Lock::acquire(&dir.join(".lock"), create)?;
        Ok((dir, lock))
    }
    fn hit(&self, point: Fault, id: Uuid) -> Result<()> {
        if self.fault == point {
            let mut f = fail(
                "partial",
                "injected interruption; explicit inspection/recovery required",
            );
            f.memory_id = Some(id);
            Err(f)
        } else {
            Ok(())
        }
    }

    pub fn resolve(&self, context: &Context) -> Result<serde_json::Value> {
        let resolution = binding::resolve(&context.working_directory, &context.authorized_boundary)
            .map_err(binding_failure)?;
        if let Some((directory, reservation)) = pending_context(context)? {
            return Ok(
                serde_json::json!({"status":"partial", "directory":directory, "memory_id":reservation.memory_id,"request_id":reservation.request_id,"reason":"unfinished_creation"}),
            );
        }
        match resolution {
            Resolution::Unbound => Ok(
                serde_json::json!({"status":"unbound", "directory":context.working_directory.canonicalize().map_err(io)?}),
            ),
            Resolution::ProjectOnly { directory } => Ok(
                serde_json::json!({"status":"unbound", "directory":directory}),
            ),
            Resolution::Declared {
                directory,
                declared_by,
            } => Ok(
                serde_json::json!({"status":"unbound", "directory":directory,"declared_by":declared_by}),
            ),
            Resolution::Bound { directory, binding } => {
                let memory = self.read(binding.memory_id)?;
                Ok(
                    serde_json::json!({"status":memory.status,"binding_directory":directory,"binding":binding,"memory":memory}),
                )
            }
        }
    }

    pub fn read(&self, id: Uuid) -> Result<State> {
        let dir = match self.directory(id, false) {
            Ok(dir) => dir,
            Err(error) if error.status == "not_found" => {
                return Ok(state(id, "not_found", "Memory directory is missing"));
            }
            Err(error) => return Err(error),
        };
        let _lock = match Lock::acquire(&dir.join(".lock"), false) {
            Ok(lock) => lock,
            Err(error) if error.io_kind.as_deref() == Some("NotFound") => {
                return Ok(state(id, "partial", "Memory lock baseline is missing"));
            }
            Err(error) => return Err(error),
        };
        self.read_locked(id, &dir)
    }
    fn read_locked(&self, id: Uuid, dir: &Path) -> Result<State> {
        if read_optional(&dir.join(PENDING))?.is_some() {
            return Ok(state(
                id,
                "partial",
                "unfinished publication; inspect recovery",
            ));
        }
        let current = read_optional(&dir.join("Memoria.md"))?;
        let head = read_optional(&dir.join(HEAD))?;
        let history = match read_optional(&dir.join("Historial.md")) {
            Ok(history) => history,
            Err(error) if error.reason.as_deref() == Some("history_corrupt") => {
                return Ok(State {
                    observed_markdown: current,
                    ..state(id, "partial", "history_corrupt")
                });
            }
            Err(error) => return Err(error),
        };
        let Some(current) = current else {
            return Ok(state(
                id,
                if history.is_none() && head.is_none() {
                    "not_found"
                } else {
                    "partial"
                },
                "current Memory is missing",
            ));
        };
        let Some(history) = history else {
            return Ok(State {
                observed_markdown: Some(current),
                ..state(
                    id,
                    "partial",
                    if head.is_none() {
                        "history_missing"
                    } else {
                        "log_baseline_incomplete"
                    },
                )
            });
        };
        if change_log::is_log(&history) {
            return Ok(
                match head
                    .as_deref()
                    .and_then(|head| change_log::baseline(id, &history, head).ok())
                {
                    Some(doc) => state_from_baseline(id, current, doc.revision, doc.raw),
                    None => State {
                        observed_markdown: Some(current),
                        ..state(id, "partial", "log_baseline_incomplete")
                    },
                },
            );
        }
        if head.is_some() {
            return Ok(State {
                observed_markdown: Some(current),
                ..state(id, "partial", "log_baseline_incomplete")
            });
        }
        let history = match History::parse(id, &history) {
            Ok(history) => history,
            Err(_) => {
                return Ok(State {
                    observed_markdown: Some(current),
                    ..state(id, "partial", "history_corrupt")
                });
            }
        };
        // Every retained snapshot must be a valid Memory document, not merely
        // plausible framing. Never adopt an invalid accepted baseline.
        for snapshot in &history.snapshots {
            match MemoryDocument::parse(id, &snapshot.markdown) {
                Ok(doc) if doc.revision == snapshot.revision => {}
                _ => {
                    return Ok(State {
                        observed_markdown: Some(current),
                        ..state(id, "partial", "history_document_invalid")
                    });
                }
            }
        }
        let last = history.latest().expect("parser requires baseline");
        Ok(state_from_baseline(
            id,
            current,
            last.revision,
            last.markdown.clone(),
        ))
    }

    pub(crate) fn with_project_guard<T>(&self, context: &Context, expected: &serde_json::Value,
        callback: impl FnOnce(&ProjectGuard<'_>) -> Result<T>) -> Result<T> {
        if self.resolve(context)? != *expected {
            return Err(fail("conflict", "selected Memory baseline changed"));
        }
        let baseline = match expected.get("status").and_then(|v| v.as_str()) {
            Some("unbound") => None,
            Some("found") => {
                let state: State = serde_json::from_value(expected["memory"].clone()).map_err(invalid)?;
                let expected = Expected { revision: state.revision.ok_or_else(|| fail("partial", "missing revision"))?,
                    accepted_markdown: state.accepted_markdown.ok_or_else(|| fail("partial", "missing accepted bytes"))? };
                let (dir, lock) = self.lock(state.memory_id, false)?;
                self.check_expected(state.memory_id, &dir, &expected, false)?;
                let document = MemoryDocument::parse(state.memory_id, &expected.accepted_markdown).map_err(invalid)?;
                Some((dir, lock, expected, document))
            }
            Some("manual_review") => return Err(fail("manual_review", "external edits require review before binding")),
            _ => return Err(fail("partial", "selected Memory is not a complete baseline")),
        };
        callback(&ProjectGuard { store: self, baseline })
    }

    /// Response-bounded tail of only the selected Memory's validated history.
    /// Baseline validation retains its existing full-file IO behavior.
    pub(crate) fn selected_history(&self, context: &Context, expected: &serde_json::Value, max_bytes: usize) -> Result<serde_json::Value> {
        self.with_project_guard(context, expected, |guard| {
            let Some((dir, _, _, _)) = &guard.baseline else {
                return Err(fail("unavailable", "selected context has no Memory history"));
            };
            let raw = read_optional(&dir.join("Historial.md"))?.ok_or_else(|| fail("unavailable", "history missing"))?;
            let mut start = raw.len().saturating_sub(max_bytes);
            while !raw.is_char_boundary(start) { start += 1; }
            let text = &raw[start..];
            Ok(serde_json::json!({"status":"found","format":if change_log::is_log(&raw) {2} else {1},
                "markdown":text,"returned_bytes":text.len(),"total_bytes":raw.len(),"truncated":start != 0}))
        })
    }

    pub fn create(&self, request: Uuid, args: Create) -> Result<Saved> {
        self.create_with_history(request, args, None)
    }

    pub fn create_log(&self, request: Uuid, args: Create, summary: &str) -> Result<Saved> {
        change_log::validate_summary(summary).map_err(invalid)?;
        self.create_with_history(request, args, Some(summary))
    }

    fn create_with_history(
        &self,
        request: Uuid,
        args: Create,
        summary: Option<&str>,
    ) -> Result<Saved> {
        request_id(request)?;
        if let Some((_, reservation)) = pending_context(&args.context)? {
            let mut failure = fail("partial", "unfinished creation; inspect, do not retry");
            failure.memory_id = Some(reservation.memory_id);
            return Err(failure);
        }
        let resolution = binding::resolve(
            &args.context.working_directory,
            &args.context.authorized_boundary,
        )
        .map_err(binding_failure)?;
        let target = match resolution {
            Resolution::Unbound => args.context.working_directory.canonicalize().map_err(io)?,
            Resolution::Declared { directory, .. } | Resolution::ProjectOnly { directory } => directory,
            Resolution::Bound { binding, .. } => {
                let mut f = fail(
                    "conflict",
                    "working context already has a binding; use read, never replace identity",
                );
                f.memory_id = Some(binding.memory_id);
                return Err(f);
            }
        };
        let _binding_lock = Lock::acquire(&target.join(".brain.toml.lock"), true)?;
        // Same directory/identity admission is rechecked after serialization.
        if let Some((_, reservation)) = pending_context(&args.context)? {
            let mut f = fail("partial", "unfinished creation; inspect, do not retry");
            f.memory_id = Some(reservation.memory_id);
            return Err(f);
        }
        let again = binding::resolve(
            &args.context.working_directory,
            &args.context.authorized_boundary,
        )
        .map_err(binding_failure)?;
        match again {
            Resolution::Bound { .. } => {
                return Err(fail("conflict", "binding appeared during creation"));
            }
            Resolution::Declared { directory, .. } if directory != target => {
                return Err(fail("conflict", "declared target changed"));
            }
            Resolution::Unbound
                if target != args.context.working_directory.canonicalize().map_err(io)? =>
            {
                return Err(fail("conflict", "parent declaration changed"));
            }
            _ => {}
        }
        let before_marker = read_optional(&target.join(binding::MARKER_NAME))?;
        if let Some(raw) = &before_marker {
            let marker = crate::project_binding::Marker::parse(raw).map_err(invalid)?;
            if marker.memory_id.is_some() { return Err(fail("conflict", "Memory binding appeared")); }
        }
        self.create_target(request, args, &target, before_marker, summary)
    }

    /// New identity for exactly one approved existing marker whose old UUID
    /// directory is truly absent. Not a general force-rebind operation.
    pub fn replace_missing(&self, request: Uuid, args: ReplaceMissing) -> Result<Saved> {
        if !args.human_approved {
            return Err(fail(
                "manual_review",
                "explicit approval of missing-reference replacement required",
            ));
        }
        request_id(request)?;
        let old = Binding::parse(&args.expected_marker).map_err(invalid)?;
        if old.memory_id != args.expected_memory_id {
            return Err(fail("conflict", "expected marker and old UUID disagree"));
        }
        let target = replacement_target(&args)?;
        let _binding_lock = Lock::acquire(&target.join(".brain.toml.lock"), true)?;
        if replacement_target(&args)? != target {
            return Err(fail("conflict", "replacement binding target changed"));
        }
        self.require_missing(args.expected_memory_id)?;
        self.create_target(
            request,
            Create {
                context: args.context,
                title: args.title,
                sections: args.sections,
                project: args.project,
                memory_subfolders: old.memory_subfolders,
            },
            &target,
            Some(args.expected_marker),
            None,
        )
    }

    fn require_missing(&self, old: Uuid) -> Result<()> {
        let state = self.read(old)?;
        if state.status != "not_found" {
            let mut error = fail(
                if state.status == "found" || state.status == "manual_review" {
                    "conflict"
                } else {
                    &state.status
                },
                "old Memory is not confirmed absent",
            );
            error.reason = state.reason;
            return Err(error);
        }
        match self.directory(old, false) {
            Err(error) if error.status == "not_found" => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(fail(
                "partial",
                "old UUID directory still has artifacts; replacement is not admitted",
            )),
        }
    }

    fn create_target(
        &self,
        request: Uuid,
        mut args: Create,
        target: &Path,
        before_marker: Option<String>,
        summary: Option<&str>,
    ) -> Result<Saved> {
        if let Some(raw) = &before_marker {
            let marker = crate::project_binding::Marker::parse(raw).map_err(invalid)?;
            if let Some(principal) = &marker.principal_project {
                let project = crate::projects::association(&self.vault, principal.project_id)?;
                if project.path != principal.project_path {
                    return Err(fail("conflict", "principal Project path changed"));
                }
                if args.project.as_ref().is_some_and(|p| *p != project) {
                    return Err(fail("conflict", "Memory Project differs from confirmed principal"));
                }
                args.project = Some(project);
            }
            args.memory_subfolders = marker.memory_subfolders;
        }
        if let Some(project) = &args.project {
            self.validate_project(project)?;
        }
        let id = Uuid::new_v4();
        let binding = Binding {
            binding_version: 1,
            memory_id: id,
            memory_subfolders: args.memory_subfolders,
        };
        binding.validate().map_err(invalid)?;
        let mut body = String::new();
        for (i, name) in CURRENT_SECTIONS.iter().enumerate() {
            body.push_str(&format!("## {name}\n{}\n", args.sections[i]));
            if i == 1
                && let Some(project) = &args.project
            {
                body.push_str(&format!("\n{}\n", project.link()));
            }
            if i == 3 {
                body.push_str(&format!("\n{}\n", history_link(id)));
            }
            body.push('\n');
        }
        let next = markdown::create_document(id, &args.title, &body, args.project.as_ref())
            .map_err(invalid)?;
        let marker = if let Some(before) = &before_marker {
            replace_marker_identity(before, id)?
        } else {
            toml::to_string(&binding).map_err(invalid)?
        };
        if let Some(before) = &before_marker
            && crate::project_binding::Marker::parse(before).map_err(invalid)?.memory_id == Some(id)
        {
            return Err(fail(
                "conflict",
                "fresh identity equals previous identity; no automatic retry",
            ));
        }
        match self.directory(id, false) {
            Err(error) if error.status == "not_found" => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(fail(
                    "conflict",
                    "allocated identity already has artifacts; do not retry automatically",
                ));
            }
        }
        let (next_history, next_head) = if let Some(summary) = summary {
            (
                change_log::append(id, None, 1, request, summary).map_err(invalid)?,
                Some(change_log::encode_head(id, request, &next).map_err(invalid)?),
            )
        } else {
            (single_history(id, request, 1, &next.raw)?, None)
        };
        let reservation = Reservation {
            version: 1,
            memory_id: id,
            request_id: request,
            before_marker: before_marker.clone(),
        };
        publish(
            &target.join(RESERVATION),
            &toml::to_string(&reservation).map_err(invalid)?,
            false,
        )?;
        // From here any error is partial and reports allocated identity.
        let result = (|| {
            self.hit(Fault::AfterReservation, id)?;
            let (dir, _lock) = self.lock(id, true)?;
            if read_optional(&dir.join("Memoria.md"))?.is_some()
                || read_optional(&dir.join(PENDING))?.is_some()
                || read_optional(&dir.join("Historial.md"))?.is_some()
                || read_optional(&dir.join(HEAD))?.is_some()
            {
                return Err(fail(
                    "conflict",
                    "allocated Memory directory already has content",
                ));
            }
            self.commit(
                &dir,
                Pending {
                    version: if next_head.is_some() { 2 } else { 1 },
                    memory_id: id,
                    request_id: request,
                    before_current: None,
                    before_history: None,
                    next_current: next.raw,
                    next_history,
                    before_head: None,
                    next_head,
                    marker: Some(marker),
                    before_marker,
                },
                Some(target),
            )
        })();
        result.map_err(|mut f: Failure| {
            if f.status != "partial" {
                f.status = "partial".into();
                f.message = "creation incomplete; inspect retained evidence, do not retry".into();
            }
            f.memory_id = Some(id);
            f
        })
    }

    pub fn update(&self, id: Uuid, request: Uuid, expected: Expected, body: &str) -> Result<Saved> {
        self.update_body(
            id,
            request,
            expected,
            |_| Ok(body.to_owned()),
            "Se actualizó el estado de trabajo.",
        )
    }

    pub fn update_sections(
        &self,
        id: Uuid,
        request: Uuid,
        expected: Expected,
        changes: &BTreeMap<String, String>,
        summary: &str,
    ) -> Result<Saved> {
        change_log::validate_summary(summary).map_err(invalid)?;
        self.update_body(
            id,
            request,
            expected,
            |document| document.with_sections(changes).map_err(invalid),
            summary,
        )
    }

    fn update_body(
        &self,
        id: Uuid,
        request: Uuid,
        expected: Expected,
        successor: impl FnOnce(&MemoryDocument) -> Result<String>,
        summary: &str,
    ) -> Result<Saved> {
        request_id(request)?;
        let (dir, _lock) = self.lock(id, false)?;
        let state = self.check_expected(id, &dir, &expected, false)?;
        let before = state.accepted_markdown.expect("checked baseline");
        let document = MemoryDocument::parse(id, &before).map_err(invalid)?;
        let body = successor(&document)?;
        if document.body() == body {
            return Ok(Saved {
                status: "unchanged".into(),
                memory_id: id,
                revision: document.revision,
                request_id: request,
                replaced_memory_id: None,
            });
        }
        let next = document.successor(&body).map_err(invalid)?;
        self.successor(&dir, request, &before, &before, next, Some(summary))
    }

    pub fn accept_external(
        &self,
        id: Uuid,
        request: Uuid,
        expected: Expected,
        observed: &str,
        approved_markdown: &str,
        approved: bool,
    ) -> Result<Saved> {
        if !approved {
            return Err(fail("manual_review", "explicit human approval required"));
        }
        request_id(request)?;
        let (dir, _lock) = self.lock(id, false)?;
        let state = self.check_expected(id, &dir, &expected, true)?;
        if state.status != "manual_review"
            || state.observed_markdown.as_deref() != Some(observed)
            || state.approval_candidate.as_deref() != Some(approved_markdown)
        {
            return Err(fail(
                "conflict",
                "approval does not match exact observed state/candidate",
            ));
        }
        let next = MemoryDocument::parse(id, approved_markdown).map_err(invalid)?;
        self.validate_association(&next)?;
        self.successor(
            &dir,
            request,
            observed,
            &expected.accepted_markdown,
            next,
            None,
        )
    }

    fn check_expected(
        &self,
        id: Uuid,
        dir: &Path,
        expected: &Expected,
        allow_manual: bool,
    ) -> Result<State> {
        let state = self.read_locked(id, dir)?;
        if !matches!(state.status.as_str(), "found" | "manual_review") {
            let mut error = fail(&state.status, "accepted baseline unavailable");
            error.reason = state.reason;
            return Err(error);
        }
        if state.revision != Some(expected.revision)
            || state.accepted_markdown.as_deref() != Some(&expected.accepted_markdown)
        {
            return Err(fail(
                "conflict",
                "expected revision/accepted bytes are stale",
            ));
        }
        if !allow_manual && state.status == "manual_review" {
            return Err(fail("manual_review", "external edits require human review"));
        }
        Ok(state)
    }
    fn successor(
        &self,
        dir: &Path,
        request: Uuid,
        before_current: &str,
        accepted: &str,
        next: MemoryDocument,
        summary: Option<&str>,
    ) -> Result<Saved> {
        let id = next.metadata.common().brain_id;
        let history = read_optional(&dir.join("Historial.md"))?
            .ok_or_else(|| fail("partial", "history missing"))?;
        let head = read_optional(&dir.join(HEAD))?;
        let (next_history, next_head) = if change_log::is_log(&history) {
            let baseline = head
                .as_deref()
                .and_then(|head| change_log::baseline(id, &history, head).ok())
                .ok_or_else(|| fail("partial", "log baseline incomplete"))?;
            if baseline.raw != accepted {
                return Err(fail("conflict", "accepted baseline changed"));
            }
            let manual_summary;
            let summary = match summary {
                Some(summary) => summary,
                None => {
                    let parts = baseline.changed_parts(&next).map_err(invalid)?;
                    manual_summary = if parts.is_empty() {
                        "Se confirmó manualmente la memoria sin cambios de contenido.".into()
                    } else {
                        format!(
                            "Se aceptaron cambios manuales en {} tras revisión explícita.",
                            parts.join(", ")
                        )
                    };
                    &manual_summary
                }
            };
            (
                change_log::append(id, Some(&history), next.revision, request, summary)
                    .map_err(invalid)?,
                Some(change_log::encode_head(id, request, &next).map_err(invalid)?),
            )
        } else {
            if head.is_some() {
                return Err(fail("partial", "unexpected accepted head"));
            }
            let mut parsed =
                History::parse(id, &history).map_err(|_| fail("partial", "history corrupt"))?;
            if parsed.latest().is_none_or(|last| last.markdown != accepted) {
                return Err(fail("conflict", "accepted baseline changed"));
            }
            parsed
                .push(AcceptedSnapshot {
                    revision: next.revision,
                    request_id: request,
                    markdown: next.raw.clone(),
                })
                .map_err(|_| fail("conflict", "revision/request already accepted"))?;
            (parsed.encode().map_err(invalid)?, None)
        };
        self.commit(
            dir,
            Pending {
                version: if next_head.is_some() { 2 } else { 1 },
                memory_id: id,
                request_id: request,
                before_current: Some(before_current.into()),
                before_history: Some(history),
                next_current: next.raw,
                next_history,
                before_head: head,
                next_head,
                marker: None,
                before_marker: None,
            },
            None,
        )
    }

    fn commit(&self, dir: &Path, pending: Pending, target: Option<&Path>) -> Result<Saved> {
        // Before creating intent, recheck exact bytes. This only serializes
        // cooperating writers; an arbitrary editor can still race this check.
        if read_optional(&dir.join("Memoria.md"))? != pending.before_current
            || read_optional(&dir.join("Historial.md"))? != pending.before_history
            || read_optional(&dir.join(HEAD))? != pending.before_head
        {
            return Err(fail(
                "conflict",
                "observed bytes changed before publication",
            ));
        }
        let raw = serde_json::to_string(&pending).map_err(invalid)?;
        publish(&dir.join(PENDING), &raw, false)?;
        let id = pending.memory_id;
        self.hit(Fault::AfterPending, id)?;
        self.finish(dir, &pending, target).map_err(|mut f| {
            f.status = "partial".into();
            f.message = "publication unconfirmed; inspect retained pending evidence".into();
            f.memory_id = Some(id);
            f
        })
    }
    fn finish(&self, dir: &Path, pending: &Pending, target: Option<&Path>) -> Result<Saved> {
        let current = read_optional(&dir.join("Memoria.md"))?;
        let history = read_optional(&dir.join("Historial.md"))?;
        let head = read_optional(&dir.join(HEAD))?;
        if head != pending.next_head && head != pending.before_head {
            return Err(fail(
                "conflict",
                "accepted head changed outside pending operation",
            ));
        }
        if current.as_ref() != Some(&pending.next_current) && current != pending.before_current {
            return Err(fail(
                "conflict",
                "current changed outside pending operation",
            ));
        }
        if history.as_ref() != Some(&pending.next_history) && history != pending.before_history {
            return Err(fail(
                "conflict",
                "history changed outside pending operation",
            ));
        }
        if current.as_ref() != Some(&pending.next_current) {
            publish(
                &dir.join("Memoria.md"),
                &pending.next_current,
                current.is_some(),
            )?;
        }
        self.hit(Fault::AfterCurrent, pending.memory_id)?;
        if history.as_ref() != Some(&pending.next_history) {
            publish(
                &dir.join("Historial.md"),
                &pending.next_history,
                history.is_some(),
            )?;
        }
        self.hit(Fault::AfterHistory, pending.memory_id)?;
        if let Some(next_head) = &pending.next_head {
            if head.as_ref() != Some(next_head) {
                publish(&dir.join(HEAD), next_head, head.is_some())?;
            }
            self.hit(Fault::AfterHead, pending.memory_id)?;
        }
        if let Some(marker) = &pending.marker {
            let target = target.ok_or_else(|| {
                fail(
                    "partial",
                    "creation recovery requires authorized working context",
                )
            })?;
            let path = target.join(binding::MARKER_NAME);
            let existing = read_optional(&path)?;
            let reservation_path = target.join(RESERVATION);
            if let Some(raw) = read_optional(&reservation_path)? {
                let expected = Reservation::from_pending(pending);
                if raw != toml::to_string(&expected).map_err(invalid)? {
                    return Err(fail("conflict", "creation reservation bytes changed"));
                }
            } else if existing.as_ref() != Some(marker) {
                return Err(fail(
                    "partial",
                    "creation receipt missing before marker publication",
                ));
            }
            if let Some(before) = &pending.before_marker {
                let old = crate::project_binding::Marker::parse(before).map_err(invalid)?.memory_id;
                if let Some(old) = old {
                let record = Reassociation {
                    version: 1,
                    request_id: pending.request_id,
                    previous_memory_id: old,
                    memory_id: pending.memory_id,
                    before_marker: before.clone(),
                    marker: marker.clone(),
                };
                let raw = serde_json::to_string(&record).map_err(invalid)?;
                match read_optional(&dir.join(REASSOCIATION))? {
                    Some(existing) if existing != raw => {
                        return Err(fail("conflict", "replacement provenance changed"));
                    }
                    Some(_) => {}
                    None => publish(&dir.join(REASSOCIATION), &raw, false)?,
                }
                }
                if existing.as_ref() != Some(marker) {
                    if existing.as_ref() != Some(before) {
                        return Err(fail(
                            "conflict",
                            "old marker was changed or removed; no restore/overwrite",
                        ));
                    }
                    if let Some(old) = old { self.require_missing(old)?; }
                    self.hit(Fault::BeforeMarker, pending.memory_id)?;
                    if read_optional(&path)?.as_ref() != Some(before) {
                        return Err(fail("conflict", "marker changed before replacement"));
                    }
                    publish(&path, marker, true)?;
                }
            } else {
                match existing {
                    Some(existing) if existing != *marker => {
                        return Err(fail("conflict", "marker differs; never replace identity"));
                    }
                    Some(_) => {}
                    None => publish(&path, marker, false)?,
                }
            }
            self.hit(Fault::AfterMarker, pending.memory_id)?;
        }
        if read_optional(&dir.join("Memoria.md"))?.as_ref() != Some(&pending.next_current)
            || read_optional(&dir.join("Historial.md"))?.as_ref() != Some(&pending.next_history)
            || read_optional(&dir.join(HEAD))? != pending.next_head
        {
            return Err(fail("conflict", "publication changed during verification"));
        }
        if let Some(marker) = &pending.marker {
            if read_optional(&target.expect("checked").join(binding::MARKER_NAME))?.as_ref()
                != Some(marker)
            {
                return Err(fail(
                    "conflict",
                    "marker changed during publication verification",
                ));
            }
            let reservation = target.expect("checked").join(RESERVATION);
            if let Some(raw) = read_optional(&reservation)? {
                let expected = Reservation::from_pending(pending);
                if raw != toml::to_string(&expected).map_err(invalid)? {
                    return Err(fail("conflict", "reservation changed before cleanup"));
                }
                fs::remove_file(reservation).map_err(io)?;
            }
            self.hit(Fault::AfterReservationCleanup, pending.memory_id)?;
        }
        fs::remove_file(dir.join(PENDING)).map_err(io)?;
        let revision = MemoryDocument::parse(pending.memory_id, &pending.next_current)
            .map_err(invalid)?
            .revision;
        Ok(Saved {
            status: "saved".into(),
            memory_id: pending.memory_id,
            revision,
            request_id: pending.request_id,
            replaced_memory_id: pending
                .before_marker
                .as_ref()
                .map(|raw| crate::project_binding::Marker::parse(raw).map(|b| b.memory_id))
                .transpose()
                .map_err(invalid)?
                .flatten(),
        })
    }

    // Only a first creation at the working directory, without an intent or
    // any vault entries, can be abandoned. Never handle replacement here.
    pub fn inspect_recovery(&self, id: Uuid, context: Option<&Context>) -> Result<Observation> {
        let dir = self.directory(id, false);
        let (directory, _lock) = match dir {
            Ok(dir) => {
                let lock = Lock::acquire(&dir.join(".lock"), false)?;
                (Some(dir), Some(lock))
            }
            Err(f) if f.status == "not_found" => (None, None),
            Err(f) => return Err(f),
        };
        self.observe(id, directory.as_deref(), context)
    }
    fn observe(
        &self,
        id: Uuid,
        dir: Option<&Path>,
        context: Option<&Context>,
    ) -> Result<Observation> {
        let (marker, reservation) = if let Some(context) = context {
            let target = recovery_target(context, id)?;
            (
                read_optional(&target.join(binding::MARKER_NAME))?,
                read_optional(&target.join(RESERVATION))?,
            )
        } else {
            (None, None)
        };
        let (history, history_bytes, history_corrupt) = match dir {
            Some(dir) => observed_history(id, &dir.join("Historial.md"))?,
            None => (None, None, false),
        };
        Ok(Observation {
            memory_id: id,
            current: dir
                .map(|d| read_optional(&d.join("Memoria.md")))
                .transpose()?
                .flatten(),
            history,
            pending: dir
                .map(|d| read_optional(&d.join(PENDING)))
                .transpose()?
                .flatten(),
            marker,
            reservation,
            reassociation: dir
                .map(|d| read_optional(&d.join(REASSOCIATION)))
                .transpose()?
                .flatten(),
            history_bytes,
            history_corrupt,
            accepted_head: dir
                .map(|d| read_optional(&d.join(HEAD)))
                .transpose()?
                .flatten(),
        })
    }

    pub fn recover(
        &self,
        id: Uuid,
        context: Option<&Context>,
        expected: Observation,
        approved: bool,
    ) -> Result<Saved> {
        if !approved {
            return Err(fail("manual_review", "explicit recovery approval required"));
        }
        let target = context.map(|ctx| recovery_target(ctx, id)).transpose()?;
        let _binding_lock = target
            .as_ref()
            .map(|path| Lock::acquire(&path.join(".brain.toml.lock"), true))
            .transpose()?;
        if let Some(context) = context
            && Some(recovery_target(context, id)?) != target
        {
            return Err(fail(
                "conflict",
                "recovery binding target changed under lock",
            ));
        }
        let (dir, _lock) = self.lock(id, false)?;
        let observed = self.observe(id, Some(&dir), context)?;
        if observed != expected || expected.memory_id != id {
            return Err(fail("conflict", "recovery observation changed"));
        }
        if observed.history_corrupt || observed.history_bytes.is_some() {
            return Err(history_corrupt());
        }
        let raw = observed.pending.ok_or_else(|| {
            fail(
                "partial",
                "no recoverable intent; preserve artifacts and request explicit reconciliation",
            )
        })?;
        let pending: Pending =
            serde_json::from_str(&raw).map_err(|_| fail("partial", "pending intent invalid"))?;
        self.validate_pending(id, &pending)?;
        if pending.marker.is_some() {
            if let Some(raw_reservation) = observed.reservation.as_deref() {
                let reservation = Reservation::parse(raw_reservation)?;
                if reservation != Reservation::from_pending(&pending) {
                    return Err(fail("conflict", "creation reservation mismatch"));
                }
            } else if observed.marker != pending.marker {
                return Err(fail(
                    "partial",
                    "creation reservation missing without completed matching marker",
                ));
            }
        }
        self.finish(&dir, &pending, target.as_deref())
            .map_err(|mut f| {
                if f.status == "unavailable" {
                    f.status = "partial".into();
                }
                f.memory_id = Some(id);
                f
            })
    }
    fn validate_pending(&self, id: Uuid, p: &Pending) -> Result<()> {
        if p.memory_id != id || p.request_id.is_nil() {
            return Err(fail("partial", "invalid pending identity"));
        }
        match (p.version, &p.next_head, &p.before_head) {
            (2, Some(_), _) => return self.validate_log_pending(id, p),
            (1, None, None) => {}
            _ => return Err(fail("partial", "invalid pending version/artifacts")),
        }
        if !matches!(
            (&p.before_current, &p.before_history, &p.marker),
            (None, None, Some(_)) | (Some(_), Some(_), None)
        ) {
            return Err(fail(
                "partial",
                "pending intent mixes creation and update baselines",
            ));
        }
        if let Some(before) = &p.before_marker {
            let old = crate::project_binding::Marker::parse(before).map_err(invalid)?;
            let next_marker = p
                .marker
                .as_deref()
                .ok_or_else(|| fail("partial", "replacement intent has no next marker"))?;
            if p.before_current.is_some()
                || p.before_history.is_some()
                || old.memory_id == Some(id)
                || replace_marker_identity(before, id)? != next_marker
            {
                return Err(fail(
                    "partial",
                    "replacement intent changes unrelated binding or reuses old identity",
                ));
            }
        }
        let next = MemoryDocument::parse(id, &p.next_current).map_err(invalid)?;
        if let crate::document::DocumentMetadata::Memory {
            brain_project: Some(project_id),
            project_link: Some(link),
            ..
        } = &next.metadata
        {
            self.validate_project_link(*project_id, link)?;
        }
        let history = History::parse(id, &p.next_history).map_err(invalid)?;
        for snapshot in &history.snapshots {
            let doc = MemoryDocument::parse(id, &snapshot.markdown).map_err(invalid)?;
            if doc.revision != snapshot.revision {
                return Err(fail(
                    "partial",
                    "pending history contains an invalid document revision",
                ));
            }
        }
        let last = history.latest().expect("parsed baseline");
        if last.markdown != p.next_current
            || last.revision != next.revision
            || last.request_id != p.request_id
        {
            return Err(fail("partial", "pending successor mismatch"));
        }
        if let Some(before) = &p.before_history {
            let old = History::parse(id, before).map_err(invalid)?;
            if history.snapshots.len() != old.snapshots.len() + 1
                || history.snapshots[..old.snapshots.len()] != old.snapshots[..]
            {
                return Err(fail("partial", "pending intent changes retained history"));
            }
        } else if p.before_current.is_some() || history.snapshots.len() != 1 || next.revision != 1 {
            return Err(fail("partial", "invalid creation intent"));
        }
        if let Some(marker) = &p.marker
            && (Binding::parse(marker).map_err(invalid)?.memory_id != id
                || p.before_current.is_some())
        {
            return Err(fail("partial", "invalid pending marker"));
        }
        Ok(())
    }
    fn validate_log_pending(&self, id: Uuid, p: &Pending) -> Result<()> {
        let bad = || fail("partial", "invalid change-log publication intent");
        let log = change_log::entries(id, &p.next_history).map_err(|_| bad())?;
        let tip = log.last().ok_or_else(bad)?;
        let next =
            change_log::baseline(id, &p.next_history, p.next_head.as_deref().ok_or_else(bad)?)
                .map_err(|_| bad())?;
        if tip.request != p.request_id || next.raw != p.next_current {
            return Err(bad());
        }
        self.validate_association(&next)?;
        match (
            &p.before_current,
            &p.before_history,
            &p.before_head,
            &p.marker,
        ) {
            (None, None, None, Some(marker)) => {
                if let Some(before) = &p.before_marker {
                    let old = crate::project_binding::Marker::parse(before).map_err(invalid)?;
                    if old.memory_id.is_some() || replace_marker_identity(before, id)? != *marker {
                        return Err(bad());
                    }
                }
                if log.len() != 1
                    || tip.revision != 1
                    || Binding::parse(marker).map_err(invalid)?.memory_id != id
                {
                    return Err(bad());
                }
            }
            (Some(before), Some(old_log), Some(old_head), None) if p.before_marker.is_none() => {
                let old = change_log::baseline(id, old_log, old_head).map_err(|_| bad())?;
                let count = change_log::entries(id, old_log).map_err(|_| bad())?.len();
                if !p.next_history.starts_with(old_log)
                    || log.len() != count + 1
                    || old.revision.checked_add(1) != Some(next.revision)
                {
                    return Err(bad());
                }
                if before != &old.raw
                    && MemoryDocument::parse(id, before)
                        .and_then(|doc| doc.approval_candidate(old.revision))
                        .map_or(true, |candidate| candidate.raw != next.raw)
                {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        }
        Ok(())
    }

    fn validate_association(&self, document: &MemoryDocument) -> Result<()> {
        if let crate::document::DocumentMetadata::Memory {
            brain_project: Some(id),
            project_link: Some(link),
            ..
        } = &document.metadata
        {
            self.validate_project_link(*id, link)?;
        }
        Ok(())
    }

    fn validate_project(&self, p: &ProjectAssociation) -> Result<()> {
        let path = relative_project(&self.vault, &p.path)?;
        let raw =
            read_optional(&path)?.ok_or_else(|| fail("invalid", "explicit Project is missing"))?;
        let metadata = markdown::project_metadata(&raw).map_err(invalid)?;
        if metadata.common().brain_id != p.brain_project {
            return Err(fail("invalid", "Project identity mismatch"));
        }
        Ok(())
    }
    fn validate_project_link(&self, id: Uuid, link: &str) -> Result<()> {
        let inner = link
            .strip_prefix("[[")
            .and_then(|s| s.strip_suffix("]]"))
            .ok_or_else(|| fail("invalid", "invalid Project link"))?;
        let (path, label) = inner
            .split_once('|')
            .ok_or_else(|| fail("invalid", "invalid Project link"))?;
        self.validate_project(&ProjectAssociation {
            brain_project: id,
            path: format!("{}.md", path.strip_suffix(".md").unwrap_or(path)),
            label: label.into(),
        })
    }
}

fn state_from_baseline(id: Uuid, current: String, revision: u64, accepted: String) -> State {
    let changed = current != accepted;
    let approval_candidate = changed
        .then(|| {
            MemoryDocument::parse(id, &current)
                .and_then(|doc| doc.approval_candidate(revision))
                .ok()
                .map(|doc| doc.raw)
        })
        .flatten();
    State {
        status: if changed { "manual_review" } else { "found" }.into(),
        memory_id: id,
        revision: Some(revision),
        accepted_markdown: Some(accepted),
        observed_markdown: changed.then_some(current),
        approval_candidate,
        reason: None,
    }
}

fn state(id: Uuid, status: &str, reason: &str) -> State {
    State {
        status: status.into(),
        memory_id: id,
        revision: None,
        accepted_markdown: None,
        observed_markdown: None,
        approval_candidate: None,
        reason: Some(reason.into()),
    }
}
fn request_id(id: Uuid) -> Result<()> {
    if id.is_nil() {
        Err(fail("invalid", "request ID must be nonnil UUID"))
    } else {
        Ok(())
    }
}
fn directory(path: &Path, create: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(fail(
            "unavailable",
            "Memory directory is not a regular directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            match fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => directory(path, false),
                Err(e) => Err(io(e)),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(fail("not_found", "Memory directory missing"))
        }
        Err(error) => Err(io(error)),
    }
}
fn history_corrupt() -> Failure {
    let mut error = fail(
        "partial",
        "history contains corrupt bytes or invalid accepted content",
    );
    error.reason = Some("history_corrupt".into());
    error
}
fn read_raw_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => Err(fail(
            "unavailable",
            "expected regular file; symlinks unsupported",
        )),
        Ok(_) => fs::read(path).map(Some).map_err(io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(error)),
    }
}
fn read_optional(path: &Path) -> Result<Option<String>> {
    read_raw_optional(path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                if path.file_name().is_some_and(|name| name == "Historial.md") {
                    history_corrupt()
                } else {
                    io(std::io::Error::from(std::io::ErrorKind::InvalidData))
                }
            })
        })
        .transpose()
}
fn observed_history(id: Uuid, path: &Path) -> Result<(Option<String>, Option<Vec<u8>>, bool)> {
    let Some(raw) = read_raw_optional(path)? else {
        return Ok((None, None, false));
    };
    match String::from_utf8(raw) {
        Ok(text) => {
            let valid = if change_log::is_log(&text) {
                // A pending intent may legitimately have published the log but
                // not yet its head. Recovery observes both exact artifacts.
                change_log::entries(id, &text).is_ok()
            } else {
                History::parse(id, &text)
                    .map(|history| {
                        history.snapshots.iter().all(|snapshot| {
                            MemoryDocument::parse(id, &snapshot.markdown)
                                .is_ok_and(|doc| doc.revision == snapshot.revision)
                        })
                    })
                    .unwrap_or(false)
            };
            Ok((Some(text), None, !valid))
        }
        Err(error) => Ok((None, Some(error.into_bytes()), true)),
    }
}

fn publish(path: &Path, raw: &str, replace: bool) -> Result<()> {
    // Trusted, stable parent required. Staged per-file publication only.
    let parent = path
        .parent()
        .ok_or_else(|| fail("invalid", "missing publication parent"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    staged.write_all(raw.as_bytes()).map_err(io)?;
    staged.flush().map_err(io)?;
    staged.as_file().sync_all().map_err(io)?;
    let existing = read_optional(path)?;
    if !replace && existing.is_some() {
        return Err(fail("conflict", "destination appeared; no clobber"));
    }
    let result = if replace {
        staged.persist(path)
    } else {
        staged.persist_noclobber(path)
    };
    result.map(|_| ()).map_err(|e| io(e.error))
}
fn single_history(id: Uuid, request: Uuid, revision: u64, current: &str) -> Result<String> {
    let mut history = History::new(id).map_err(invalid)?;
    history
        .push(AcceptedSnapshot {
            revision,
            request_id: request,
            markdown: current.into(),
        })
        .map_err(invalid)?;
    history.encode().map_err(invalid)
}
fn pending_context(context: &Context) -> Result<Option<(PathBuf, Reservation)>> {
    // First perform the normal scope/symlink checks; no broader traversal.
    binding::resolve(&context.working_directory, &context.authorized_boundary)
        .map_err(binding_failure)?;
    let boundary = context.authorized_boundary.canonicalize().map_err(io)?;
    let mut dir = context.working_directory.canonicalize().map_err(io)?;
    loop {
        if let Some(raw) = read_optional(&dir.join(RESERVATION))? {
            return Ok(Some((dir, Reservation::parse(&raw)?)));
        }
        // A closer own marker masks any farther context/reservation.
        if read_optional(&dir.join(binding::MARKER_NAME))?.is_some() || dir == boundary {
            return Ok(None);
        }
        dir = dir
            .parent()
            .ok_or_else(|| fail("invalid", "invalid boundary traversal"))?
            .into();
    }
}
fn recovery_target(context: &Context, id: Uuid) -> Result<PathBuf> {
    if let Some((dir, reservation)) = pending_context(context)? {
        if reservation.memory_id != id {
            return Err(fail("conflict", "reservation belongs to another Memory"));
        }
        return Ok(dir);
    }
    match binding::resolve(&context.working_directory, &context.authorized_boundary)
        .map_err(binding_failure)?
    {
        Resolution::Bound { directory, binding } if binding.memory_id == id => Ok(directory),
        _ => Err(fail(
            "conflict",
            "recovery context has no matching binding/reservation",
        )),
    }
}
fn replacement_target(args: &ReplaceMissing) -> Result<PathBuf> {
    if let Some((_, reservation)) = pending_context(&args.context)? {
        let mut error = fail(
            "partial",
            "unfinished creation/replacement requires recovery, not a new identity",
        );
        error.memory_id = Some(reservation.memory_id);
        return Err(error);
    }
    match binding::resolve(
        &args.context.working_directory,
        &args.context.authorized_boundary,
    )
    .map_err(binding_failure)?
    {
        Resolution::Bound { directory, binding }
            if directory == args.binding_directory
                && binding.memory_id == args.expected_memory_id =>
        {
            if read_optional(&directory.join(binding::MARKER_NAME))?.as_deref()
                != Some(&args.expected_marker)
            {
                return Err(fail("conflict", "expected old marker bytes changed"));
            }
            Ok(directory)
        }
        _ => Err(fail(
            "conflict",
            "replacement must target the actual authorized existing marker, not a declaration/descendant",
        )),
    }
}
fn replace_marker_identity(before: &str, next: Uuid) -> Result<String> {
    #[derive(Deserialize)]
    struct Span {
        memory_id: toml::Spanned<String>,
    }
    let old = crate::project_binding::Marker::parse(before).map_err(invalid)?;
    let mut result = before.to_owned();
    if old.memory_id.is_some() {
        let span: Span = toml::from_str(before).map_err(invalid)?;
        result.replace_range(span.memory_id.span(), &format!("\"{next}\""));
    } else {
        result.insert_str(0, &format!("memory_id = \"{next}\"\n"));
    }
    let parsed = crate::project_binding::Marker::parse(&result).map_err(invalid)?;
    if parsed.memory_id != Some(next) || parsed.memory_subfolders != old.memory_subfolders
        || parsed.principal_project != old.principal_project {
        return Err(fail(
            "invalid",
            "marker identity replacement did not preserve declarations",
        ));
    }
    Ok(result)
}
fn relative_project(vault: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || path.contains(['\\', ':', '#', '|', '[', ']', '\0'])
        || !path.ends_with(".md")
        || !relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        || relative.starts_with("05-memorias")
    {
        return Err(fail(
            "invalid",
            "Project requires explicit safe path outside 05-memorias",
        ));
    }
    let mut joined = vault.to_path_buf();
    for component in relative.components() {
        joined.push(component);
        if fs::symlink_metadata(&joined)
            .map_err(io)?
            .file_type()
            .is_symlink()
        {
            return Err(fail("unavailable", "Project path contains symlink"));
        }
    }
    Ok(joined)
}
