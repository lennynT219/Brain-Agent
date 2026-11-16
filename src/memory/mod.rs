//! Bound, file-only project-memory archive. See `docs/PROJECT_MEMORY_STORE.md`.
//! Binding authorization and write-once storage are trusted-caller preconditions.

mod store;

use crate::document::DocumentMetadata;
use serde::{Deserialize, Serialize};
use std::{fmt, io, path::PathBuf};
use uuid::Uuid;

pub use store::MemoryStore;

pub const COLLECTION: &str = "stela-shell";
pub const STORE_VERSION: u32 = 1;
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_READ_BYTES: usize = 64 * 1024;
pub const MAX_REVISIONS: usize = 128;
pub const MAX_GRAPH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REVISION: u64 = i64::MAX as u64;
pub type Result<T> = std::result::Result<T, Error>;

/// Not proof of consent or project existence in a real vault. No config discovery.
#[derive(Debug, Clone)]
pub struct Binding {
    root: PathBuf,
    project_id: Uuid,
}

impl Binding {
    pub fn new(root: PathBuf, project: &DocumentMetadata, project_id: Uuid) -> Result<Self> {
        if !matches!(project, DocumentMetadata::Project(_))
            || project.common().brain_id != project_id
            || project.validate().is_err()
            || !root.is_absolute()
        {
            return Err(Error::InvalidBinding);
        }
        store::directory(&root)?;
        Ok(Self { root, project_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    SourceObservation,
    UserDecision,
    AgentInference,
}

/// References are inert attribution, not fetched evidence or verified facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub reference: String,
    pub locator: Option<String>,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub title: String,
    pub claim_kind: ClaimKind,
    pub session: String,
    pub observed_at_unix_ms: u64,
    pub sources: Vec<Source>,
}

impl Metadata {
    fn validate(&self) -> Result<()> {
        let text = |s: &str| !s.trim().is_empty() && s.len() <= 2048;
        if !text(&self.title)
            || !text(&self.session)
            || self.observed_at_unix_ms > MAX_REVISION
            || self.sources.len() > 32
            || self.sources.iter().any(|s| {
                !text(&s.reference)
                    || s.locator.as_deref().is_some_and(|s| !text(s))
                    || s.revision.as_deref().is_some_and(|s| !text(s))
            })
        {
            return Err(Error::InvalidInput("metadata/provenance"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub store_version: u32,
    pub project_id: Uuid,
    pub collection: String,
    pub memory_id: Uuid,
    pub revision: u64,
    pub parent: Option<u64>,
    pub source_view: String,
    pub base_revision: u64,
    pub payload_len: u64,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewState {
    Base,
    Stale,
    Diverged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionState {
    Current,
    Historical,
    Unresolved,
}

#[derive(Debug)]
pub struct ReadResult {
    pub header: Header,
    pub bytes: Vec<u8>,
    pub total_bytes: usize,
    pub truncated: bool,
    pub heads: Vec<u64>,
    pub state: RevisionState,
    pub view: ViewState,
    pub needs_review: bool,
}

/// The record is published even when the post-publication view check finds drift.
#[derive(Debug)]
pub struct Publication {
    pub revision: u64,
    pub view: ViewState,
    pub needs_review: bool,
}

#[derive(Debug)]
pub enum Error {
    InvalidBinding,
    InvalidInput(&'static str),
    NotFound,
    UnsafePath(PathBuf),
    Busy,
    AlreadyExists,
    Collision(u64),
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    NeedsReview {
        heads: Vec<u64>,
        view: ViewState,
    },
    Malformed(&'static str),
    UnsupportedVersion(u32),
    Limit(&'static str),
    Overflow,
    Io(io::Error),
    /// No success/rollback claim: publication may have occurred. Inspect history.
    PublicationUncertain {
        revision: u64,
        source: io::Error,
    },
    /// Definite publication; subsequent sync/observation failed. Never roll back.
    AfterPublication {
        revision: u64,
        source: Box<Error>,
    },
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(error)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "project memory: {self:?}")
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(source) | Self::PublicationUncertain { source, .. } => Some(source),
            Self::AfterPublication { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
