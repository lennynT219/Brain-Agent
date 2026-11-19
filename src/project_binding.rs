//! Unified exact-directory Project binding. Reading never migrates legacy markers.
use crate::memory::binding::Binding;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Principal {
    pub project_id: Uuid,
    pub project_path: String,
    pub allow_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Marker {
    pub binding_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_subfolders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_project: Option<Principal>,
}

pub(crate) fn valid_project_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 3 && parts[0] == "01-proyectos" && parts[2].ends_with(".md")
        && parts.iter().all(|part| safe_component(part))
}

pub(crate) fn safe_component(part: &str) -> bool {
    !part.trim().is_empty() && part != "." && part != ".."
        && !part.ends_with(['.', ' '])
        && !part.chars().any(|c| c.is_control() || "\\/:*?\"<>|[]#".contains(c))
        && !matches!(part.split('.').next().unwrap_or("").to_ascii_uppercase().as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9")
}

/// Windows reserves device stems even with extensions or spaces before the
/// first dot, including superscript digits for COM/LPT.
pub(crate) fn reserved_windows_stem(title: &str) -> bool {
    let stem = title.split('.').next().unwrap_or("").trim_end_matches([' ', '.']).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || ["COM", "LPT"].iter().any(|&prefix| stem.strip_prefix(prefix).is_some_and(|suffix|
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")))
}

impl Marker {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let marker: Self = toml::from_str(raw)?;
        if !matches!(marker.binding_version, 1 | 2)
            || marker.memory_id.is_some_and(|id| id.is_nil())
            || (marker.binding_version == 1 && (marker.memory_id.is_none() || marker.principal_project.is_some()))
            || (marker.binding_version == 2 && marker.principal_project.is_none())
        {
            bail!("invalid unified binding identity/version");
        }
        // Reuse the existing portable child-declaration validation.
        Binding { binding_version: 1, memory_id: marker.memory_id.unwrap_or_else(|| Uuid::from_u128(1)),
            memory_subfolders: marker.memory_subfolders.clone() }.validate()?;
        if let Some(project) = &marker.principal_project
            && (project.project_id.is_nil() || !valid_project_path(&project.project_path))
        {
            bail!("invalid principal Project");
        }
        Ok(marker)
    }

    pub(crate) fn memory_binding(&self) -> Option<Binding> {
        self.memory_id.map(|memory_id| Binding { binding_version: 1, memory_id,
            memory_subfolders: self.memory_subfolders.clone() })
    }
}

/// An explicit bind adds owned v2 fields without reserializing existing bytes.
/// Ambiguous TOML spellings are rejected rather than normalized.
pub(crate) fn bind_bytes(raw: Option<&str>, principal: Principal) -> Result<String> {
    let Some(raw) = raw else {
        return Ok(toml::to_string(&Marker { binding_version: 2, memory_id: None,
            memory_subfolders: vec![], principal_project: Some(principal) })?);
    };
    let before = Marker::parse(raw)?;
    if before.principal_project.as_ref() == Some(&principal) { return Ok(raw.to_owned()); }
    #[derive(Deserialize)]
    struct Fields { binding_version: toml::Spanned<i64>, principal_project: Option<toml::Spanned<toml::Value>> }
    let fields: Fields = toml::from_str(raw)?;
    if fields.principal_project.is_some() {
        // Owned table replacement is deliberately restricted to our emitted
        // final table; do not absorb unrelated later tables or comments.
        let header = "[principal_project]";
        let starts: Vec<_> = raw.match_indices(header).map(|(i, _)| i).collect();
        if starts.len() != 1 { bail!("principal table spelling requires review"); }
        let start = starts[0];
        let tail = &raw[start + header.len()..];
        if tail.lines().any(|l| l.trim_start().starts_with('[') || l.contains('#')) {
            bail!("principal table contains material requiring review");
        }
        let replacement = format!("[principal_project]\n{}", toml::to_string(&principal)?);
        let mut next = raw[..start].to_owned();
        next.push_str(&replacement);
        let after = Marker::parse(&next)?;
        if after.memory_id != before.memory_id || after.memory_subfolders != before.memory_subfolders {
            bail!("binding changed Memory fields");
        }
        return Ok(next);
    }
    let mut next = raw.to_owned();
    next.replace_range(fields.binding_version.span(), "2");
    if !next.ends_with('\n') { next.push('\n'); }
    next.push_str(&format!("\n[principal_project]\n{}", toml::to_string(&principal)?));
    Marker::parse(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn principal(n: u128) -> Principal {
        Principal { project_id: Uuid::from_u128(n), project_path: format!("01-proyectos/P{n}/Proyecto.md"), allow_write: true }
    }
    #[test]
    fn explicit_bind_preserves_memory_marker_bytes_and_declarations() {
        let id = Uuid::from_u128(10);
        let old = format!("# comentario humano\r\nbinding_version = 1 # versión\r\n\"memory_id\" = '{id}'\r\nmemory_subfolders = [ 'child' ]\r\n");
        let next = bind_bytes(Some(&old), principal(1)).unwrap();
        assert!(next.starts_with(&old.replace("binding_version = 1", "binding_version = 2")));
        let parsed = Marker::parse(&next).unwrap();
        assert_eq!(parsed.memory_id, Some(id));
        assert_eq!(parsed.memory_subfolders, vec!["child"]);
        assert_eq!(parsed.principal_project, Some(principal(1)));
        assert_eq!(bind_bytes(Some(&next), principal(1)).unwrap(), next);
        let rebound = bind_bytes(Some(&next), principal(2)).unwrap();
        assert_eq!(Marker::parse(&rebound).unwrap().memory_id, Some(id));
    }
    #[test]
    fn v2_project_only_is_explicit_and_malformed_markers_fail_closed() {
        let raw = bind_bytes(None, principal(1)).unwrap();
        assert!(Marker::parse(&raw).unwrap().memory_id.is_none());
        assert!(Marker::parse("binding_version=2\n").is_err());
        assert!(Marker::parse("binding_version=1\n").is_err());
        assert!(!valid_project_path("01-proyectos/../Escape.md"));
        assert!(!valid_project_path("01-proyectos/NUL/Proyecto.md"));
    }
}
