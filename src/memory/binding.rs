//! Read-only resolution inside an explicit caller-authorized directory boundary.
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub const MARKER_NAME: &str = ".brain.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub binding_version: u32,
    #[serde(deserialize_with = "uuid_string")]
    pub memory_id: Uuid,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_subfolders: Vec<String>,
}

fn uuid_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let value = String::deserialize(deserializer)?;
    Uuid::parse_str(&value).map_err(serde::de::Error::custom)
}

impl Binding {
    pub fn parse(input: &str) -> Result<Self> {
        let marker = crate::project_binding::Marker::parse(input)?;
        marker.memory_binding().context("marker has no Memory identity")
    }

    pub fn validate(&self) -> Result<()> {
        if self.binding_version != 1 || self.memory_id.is_nil() {
            bail!("unsupported binding version or nil Memory identity");
        }
        let mut paths = std::collections::HashSet::new();
        for path in &self.memory_subfolders {
            // Portable explicit relative paths, not globs or traversal syntax.
            if path.contains(['\\', ':', '*', '?', '[', ']', '\0'])
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || !Path::new(path)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                || !paths.insert(path)
            {
                bail!("invalid or duplicate declared relative subfolder");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Unbound,
    /// A confirmed exact-context Project without a Memory yet.
    ProjectOnly { directory: PathBuf },
    Bound {
        directory: PathBuf,
        binding: Binding,
    },
    /// An explicit lazy child context, not a newly allocated Memory identity.
    Declared {
        directory: PathBuf,
        declared_by: PathBuf,
    },
}

/// Resolve the nearest enclosing marker, then its most specific applicable
/// child declaration. An own marker at a closer level wins over declarations
/// on a more distant marker. Missing markers are normal; other I/O errors are
/// propagated with their original sources. No vault content is consulted.
///
/// Existing directories are canonicalized and symlink path components below
/// the boundary are rejected. This is not a sandbox against concurrent path
/// replacement; the caller must control directory traversal and mutation.
pub fn resolve(working_directory: &Path, authorized_boundary: &Path) -> Result<Resolution> {
    let boundary = authorized_boundary
        .canonicalize()
        .context("cannot resolve authorized boundary")?;
    let working = working_directory
        .canonicalize()
        .context("cannot resolve working directory")?;
    if !boundary.is_dir() || !working.is_dir() || !working.starts_with(&boundary) {
        bail!("working directory is outside the authorized directory boundary");
    }
    // Inspect lexical input too: canonicalization alone could silently traverse
    // a symlink outside the boundary and back into it.
    let lexical_working = if working_directory.is_absolute() {
        working_directory.to_path_buf()
    } else {
        std::env::current_dir()?.join(working_directory)
    };
    if lexical_working
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        bail!("working path must not contain parent traversal");
    }
    let lexical_boundary = if authorized_boundary.is_absolute() {
        authorized_boundary.to_path_buf()
    } else {
        std::env::current_dir()?.join(authorized_boundary)
    };
    let relative = lexical_working
        .strip_prefix(&lexical_boundary)
        .context("working path must be lexically within the authorized boundary")?;
    let mut checked = lexical_boundary;
    for part in relative.components() {
        checked.push(part);
        if fs::symlink_metadata(&checked)?.file_type().is_symlink() {
            bail!("symlink traversal below authorized boundary is unsupported");
        }
    }
    let mut directory = working.as_path();
    loop {
        let marker = directory.join(MARKER_NAME);
        match fs::symlink_metadata(&marker) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    bail!("binding marker is not a regular file");
                }
                let input = fs::read_to_string(&marker).context("cannot read binding marker")?;
                let parsed_marker = crate::project_binding::Marker::parse(&input)?;
                if directory != working {
                    let mut selected: Option<PathBuf> = None;
                    for child in &parsed_marker.memory_subfolders {
                        let candidate = directory.join(child);
                        if working.starts_with(&candidate)
                            && selected.as_ref().is_none_or(|old| {
                                candidate.components().count() > old.components().count()
                            })
                        {
                            selected = Some(candidate);
                        }
                    }
                    if let Some(directory) = selected {
                        return Ok(Resolution::Declared {
                            directory,
                            declared_by: marker,
                        });
                    }
                }
                let Some(binding) = parsed_marker.memory_binding() else {
                    return Ok(Resolution::ProjectOnly { directory: directory.to_path_buf() });
                };
                return Ok(Resolution::Bound {
                    directory: directory.to_path_buf(),
                    binding,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect binding marker"),
        }
        if directory == boundary {
            return Ok(Resolution::Unbound);
        }
        directory = directory
            .parent()
            .context("cannot walk authorized boundary")?;
    }
}
