//! Validate owned fields while retaining the original Markdown, not a Serde
//! round-trip of unknown properties/comments. No include/property resolver.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::history_link;
use crate::document::DocumentMetadata;

#[derive(Debug, Clone)]
pub struct MemoryDocument {
    pub metadata: DocumentMetadata,
    pub raw: String,
    pub revision: u64,
    body_start: usize,
    frontmatter_end: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SectionEdit {
    pub section: String,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug)]
pub(super) enum EditFailure {
    NotFound,
    Ambiguous,
    Overlap,
}

impl EditFailure {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::NotFound => "EDIT_NOT_FOUND",
            Self::Ambiguous => "EDIT_AMBIGUOUS",
            Self::Overlap => "EDITS_OVERLAP",
        }
    }
}

impl std::fmt::Display for EditFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}
impl std::error::Error for EditFailure {}

fn section_content_start(body: &str, start: usize) -> usize {
    let section = &body[start..];
    section.find(['\r', '\n']).map_or(body.len(), |n| {
        start + n + if section[n..].starts_with("\r\n") { 2 } else { 1 }
    })
}

/// Split off the last content line's terminator and any following complete
/// blank lines. The H2 terminator is not part of `content`.
fn section_separator(content: &str) -> (&str, &str) {
    let mut end = content.len();
    while end != 0 {
        let line = &content[..end];
        let Some(text) = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix(['\r', '\n']))
        else {
            // A next canonical H2 is at a line start, so this should not occur.
            return ("", &content[end..]);
        };
        let start = text.rfind(['\r', '\n']).map_or(0, |i| i + 1);
        if !text[start..].bytes().all(|b| matches!(b, b' ' | b'\t')) {
            return (&content[text.len()..end], &content[end..]);
        }
        end = start;
    }
    ("", content)
}

fn frontmatter(input: &str) -> Result<(&str, usize, usize)> {
    let opening = if input.starts_with("---\r\n") {
        5
    } else if input.starts_with("---\n") {
        4
    } else {
        bail!("missing YAML frontmatter");
    };
    let mut offset = opening;
    for line in input[opening..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok((&input[opening..offset], offset + line.len(), offset));
        }
        offset += line.len();
    }
    bail!("unterminated YAML frontmatter")
}

impl MemoryDocument {
    pub fn parse(expected_id: Uuid, input: &str) -> Result<Self> {
        let (yaml, body_start, frontmatter_end) = frontmatter(input)?;
        let metadata: DocumentMetadata =
            serde_saphyr::from_str(yaml).context("invalid Memory YAML")?;
        metadata.validate().map_err(anyhow::Error::msg)?;
        let DocumentMetadata::Memory {
            memory_revision, ..
        } = &metadata
        else {
            bail!("document is not Memory");
        };
        let revision = *memory_revision;
        if metadata.common().brain_id != expected_id {
            bail!("Memory identity mismatch");
        }
        let active = super::syntax::active_relations(&input[body_start..])?;
        if !active.contains(&history_link(expected_id)) {
            bail!("missing qualified history relation");
        }
        if let DocumentMetadata::Memory {
            project_link: Some(link),
            ..
        } = &metadata
            && !active.contains(link)
        {
            bail!("Project relation must also be a body wikilink");
        }
        Ok(Self {
            metadata,
            raw: input.into(),
            revision,
            body_start,
            frontmatter_end,
        })
    }

    pub fn body(&self) -> &str {
        &self.raw[self.body_start..]
    }

    /// Apply whole section contents (without their H2s) at validated Markdown
    /// boundaries. Verify replacements did not hide or introduce a boundary.
    pub(super) fn with_sections(
        &self,
        changes: &std::collections::BTreeMap<String, String>,
    ) -> Result<String> {
        use super::CURRENT_SECTIONS;
        let body = self.body();
        if changes.is_empty()
            || changes
                .keys()
                .any(|k| !CURRENT_SECTIONS.contains(&k.as_str()))
        {
            bail!("provide one or more named canonical sections");
        }
        let (_, starts) = super::syntax::topology(body)?;
        let mut result = body[..starts[0]].to_owned();
        let mut expected = Vec::with_capacity(starts.len());
        for (i, start) in starts.iter().copied().enumerate() {
            expected.push(result.len());
            let end = starts.get(i + 1).copied().unwrap_or(body.len());
            let original = &body[start..end];
            let Some(change) = changes.get(CURRENT_SECTIONS[i]) else {
                result.push_str(original);
                continue;
            };
            let line_end = original.find(['\r', '\n']);
            let (heading, newline) = match line_end {
                Some(n) if original[n..].starts_with("\r\n") => (&original[..n], "\r\n"),
                Some(n) => (&original[..n], &original[n..n + 1]),
                None => (original, "\n"),
            };
            if line_end.is_none() && change.is_empty() {
                result.push_str(original);
                continue;
            }
            result.push_str(heading);
            result.push_str(newline);
            result.push_str(change);
            if i + 1 < starts.len() && !change.ends_with(['\r', '\n']) {
                let (terminator, blanks) =
                    section_separator(&original[heading.len() + newline.len()..]);
                // Do not silently fuse an inherited blank line (or a leading
                // LF in the replacement) with an isolated CR on the H2.
                let following = if change.is_empty() { blanks } else { change };
                if newline == "\r" && following.starts_with('\n') {
                    bail!("section replacement would fuse two line endings");
                }
                if !change.is_empty() {
                    result.push_str(if terminator.is_empty() {
                        newline
                    } else {
                        terminator
                    });
                }
                result.push_str(blanks);
            }
        }
        let (_, actual) = super::syntax::topology(&result)?;
        if actual != expected {
            bail!("section replacement changed the canonical heading boundaries");
        }
        Ok(result)
    }

    /// Resolve literal ranges on one original body; never search replacement text.
    pub(super) fn with_section_edits(&self, edits: &[SectionEdit]) -> Result<String> {
        use super::CURRENT_SECTIONS;
        if !(1..=32).contains(&edits.len()) {
            bail!("provide 1 to 32 section edits");
        }
        let body = self.body();
        let (_, headings) = super::syntax::topology(body)?;
        let mut resolved = Vec::with_capacity(edits.len());
        for edit in edits {
            let i = CURRENT_SECTIONS
                .iter()
                .position(|&name| name == edit.section)
                .context("unknown canonical section")?;
            let first_char = edit.old_text.chars().next().context("empty edit anchor")?;
            let start = section_content_start(body, headings[i]);
            let end = headings.get(i + 1).copied().unwrap_or(body.len());
            let content = &body[start..end];
            let hit = content.find(&edit.old_text).ok_or(EditFailure::NotFound)?;
            // Advance only one character so overlapping occurrences count too.
            if content[hit + first_char.len_utf8()..].contains(&edit.old_text) {
                return Err(EditFailure::Ambiguous.into());
            }
            let start = start + hit;
            resolved.push((start, start + edit.old_text.len(), edit.new_text.as_str()));
        }
        resolved.sort_unstable_by_key(|&(start, _, _)| start);
        if resolved.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(EditFailure::Overlap.into());
        }

        let mut result = String::with_capacity(body.len());
        let mut edits = resolved.into_iter().peekable();
        let mut expected = Vec::with_capacity(headings.len());
        let mut cursor = 0;
        for boundary in headings.iter().copied().chain(std::iter::once(body.len())) {
            while let Some(&(start, end, text)) = edits.peek() {
                if start >= boundary {
                    break;
                }
                result.push_str(&body[cursor..start]);
                result.push_str(text);
                cursor = end;
                edits.next();
            }
            result.push_str(&body[cursor..boundary]);
            expected.push(result.len());
            cursor = boundary;
        }
        expected.pop(); // EOF is not a heading.
        let (_, actual) = super::syntax::topology(&result)?;
        if actual != expected
            || headings.iter().zip(&actual).any(|(&before, &after)| {
                section_content_start(body, before) - before
                    != section_content_start(&result, after) - after
            })
        {
            bail!("section edits changed the canonical heading boundaries");
        }
        Ok(result)
    }

    pub(super) fn changed_parts(&self, next: &Self) -> Result<Vec<&'static str>> {
        let before = self.body();
        let after = next.body();
        let (_, first) = super::syntax::topology(before)?;
        let (_, second) = super::syntax::topology(after)?;
        let mut parts = Vec::new();
        let same_header = replace_revision(
            &self.raw[..self.body_start],
            next.revision,
            self.frontmatter_end,
        )
        .is_ok_and(|header| header == next.raw[..next.body_start]);
        if !same_header || before[..first[0]] != after[..second[0]] {
            parts.push("metadatos o preámbulo");
        }
        for (i, name) in super::CURRENT_SECTIONS.iter().enumerate() {
            if before[first[i]..first.get(i + 1).copied().unwrap_or(before.len())]
                != after[second[i]..second.get(i + 1).copied().unwrap_or(after.len())]
            {
                parts.push(name);
            }
        }
        Ok(parts)
    }

    /// Normal updates change only body and the digits of an owned revision
    /// scalar. Unsupported YAML spellings fail; they are never normalized.
    pub fn successor(&self, body: &str) -> Result<Self> {
        let next = self.revision.checked_add(1).context("revision overflow")?;
        let raw =
            replace_revision(&self.raw[..self.body_start], next, self.frontmatter_end)? + body;
        let parsed = Self::parse(self.metadata.common().brain_id, &raw)?;
        if parsed.revision != next {
            bail!("revision replacement failed");
        }
        Ok(parsed)
    }

    /// Explicit Project association changes only owned scalars, revision and
    /// a delimited body relation. Existing human references remain untouched.
    pub(crate) fn with_project(&self, project: &ProjectAssociation) -> Result<Self> {
        let DocumentMetadata::Memory { brain_project, project_link, .. } = &self.metadata else {
            bail!("not a Memory");
        };
        let link = project.link();
        if *brain_project == Some(project.brain_project) && project_link.as_deref() == Some(&link) {
            return Ok(self.clone());
        }
        let prefix = &self.raw[..self.frontmatter_end];
        let mut header = String::new();
        let mut ids = 0;
        let mut links = 0;
        for line in prefix.split_inclusive('\n') {
            if line.starts_with("brain_project:") || line.starts_with("project_link:") {
                if line.contains('#') || line.trim_end().ends_with(['|', '>']) {
                    bail!("Project scalar spelling requires explicit review");
                }
                if line.starts_with("brain_project:") { ids += 1; } else { links += 1; }
            } else { header.push_str(line); }
        }
        if ids > 1 || links > 1 || (brain_project.is_some() && (ids != 1 || links != 1)) {
            bail!("Project scalar spelling requires explicit review");
        }
        // Ensure a block scalar/quoted key was not accidentally consumed.
        let parsed: DocumentMetadata = serde_saphyr::from_str(header.trim_start_matches("---").trim_start())?;
        if let DocumentMetadata::Memory { brain_project, project_link, .. } = parsed
            && (brain_project.is_some() || project_link.is_some()) {
            bail!("Project scalars could not be isolated");
        }
        let newline = if self.raw.starts_with("---\r\n") { "\r\n" } else { "\n" };
        if !header.ends_with('\n') { header.push_str(newline); }
        header.push_str(&format!("brain_project: {}{newline}project_link: {}{newline}",
            project.brain_project, serde_json::to_string(&link)?));
        let start = "<!-- brain:principal-project -->";
        let end = "<!-- /brain:principal-project -->";
        let block = format!("{start}{newline}{link}{newline}{end}");
        let mut body = self.body().to_owned();
        match (body.find(start), body.find(end)) {
            (None, None) => {
                let (_, headings) = super::syntax::topology(&body)?;
                body.insert_str(headings[2], &format!("{block}{newline}{newline}"));
            }
            (Some(a), Some(b)) if a < b && body.matches(start).count() == 1 && body.matches(end).count() == 1 => {
                let (_, headings) = super::syntax::topology(&body)?;
                let genuine = |position: usize, marker: &str| {
                    position > 0 && body.as_bytes()[position - 1] == b'\n'
                        && pulldown_cmark::Parser::new(&body).into_offset_iter().any(|(event, range)| {
                            matches!(event, pulldown_cmark::Event::Html(ref text) | pulldown_cmark::Event::InlineHtml(ref text)
                                if range.start == position && text.trim_end_matches(['\r', '\n']) == marker)
                        })
                };
                if a < headings[1] || b >= headings[2] || !genuine(a, start) || !genuine(b, end) {
                    bail!("managed Project relation is not an owned top-level block");
                }
                let old = &body[a + start.len()..b];
                if project_link.as_deref() != Some(old.trim()) {
                    bail!("managed Project relation was edited");
                }
                body.replace_range(a..b + end.len(), &block);
            }
            _ => bail!("managed Project relation is ambiguous"),
        }
        let revision = self.revision.checked_add(1).context("revision overflow")?;
        let header_end = header.len();
        let header = replace_revision(&header, revision, header_end)?;
        Self::parse(self.metadata.common().brain_id, &format!("{header}---{newline}{body}"))
    }

    /// Candidate for explicit review. Preserves all external bytes except the
    /// displayed revision digits, unless the user already advanced them.
    pub fn approval_candidate(&self, accepted_revision: u64) -> Result<Self> {
        let next = accepted_revision
            .checked_add(1)
            .context("revision overflow")?;
        if self.revision == next {
            return Ok(self.clone());
        }
        if self.revision != accepted_revision {
            bail!("external revision is neither baseline nor successor");
        }
        self.successor(self.body())
    }
}

fn replace_revision(prefix: &str, revision: u64, end: usize) -> Result<String> {
    let mut result = String::new();
    let mut found = false;
    let mut offset = 0;
    for line in prefix.split_inclusive('\n') {
        if offset < end
            && let Some(rest) = line.strip_prefix("memory_revision:")
        {
            let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            let digits = rest[spaces..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if found || digits == 0 {
                bail!("revision scalar requires explicit reviewed spelling");
            }
            let tail = &rest[spaces + digits..];
            let trimmed = tail.trim_start_matches([' ', '\t']);
            if !trimmed.trim_end_matches(['\r', '\n']).is_empty() && !trimmed.starts_with('#') {
                bail!("revision scalar requires plain unsigned decimal syntax");
            }
            result.push_str("memory_revision:");
            result.push_str(&rest[..spaces]);
            result.push_str(&revision.to_string());
            result.push_str(tail);
            found = true;
            offset += line.len();
            continue;
        }
        result.push_str(line);
        offset += line.len();
    }
    if !found {
        bail!("revision scalar requires explicit reviewed spelling");
    }
    Ok(result)
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectAssociation {
    pub brain_project: Uuid,
    /// Qualified vault-relative Markdown path, supplied by the trusted caller.
    pub path: String,
    pub label: String,
}

impl ProjectAssociation {
    pub fn link(&self) -> String {
        format!(
            "[[{}|{}]]",
            self.path.strip_suffix(".md").unwrap_or(&self.path),
            self.label
        )
    }
}

pub fn create_document(
    id: Uuid,
    title: &str,
    body: &str,
    project: Option<&ProjectAssociation>,
) -> Result<MemoryDocument> {
    let metadata = DocumentMetadata::Memory {
        common: crate::document::CommonMetadata {
            brain_version: 1,
            brain_id: id,
            title: title.into(),
        },
        memory_revision: 1,
        brain_project: project.map(|p| p.brain_project),
        project_link: project.map(ProjectAssociation::link),
    };
    metadata.validate().map_err(anyhow::Error::msg)?;
    let yaml = serde_saphyr::to_string(&metadata).context("cannot encode Memory metadata")?;
    let raw = format!("---\n{yaml}---\n{body}");
    MemoryDocument::parse(id, &raw)
}

pub(crate) fn project_metadata(input: &str) -> Result<DocumentMetadata> {
    let (yaml, _, _) = frontmatter(input)?;
    let metadata: DocumentMetadata = serde_saphyr::from_str(yaml)?;
    metadata.validate().map_err(anyhow::Error::msg)?;
    if !matches!(metadata, DocumentMetadata::Project(_)) {
        bail!("relation target is not Project");
    }
    Ok(metadata)
}

#[cfg(test)]
mod whole_section_tests {
    use super::*;

    #[test]
    fn preserve_the_exact_separator_unless_a_terminator_is_explicit() {
        let id = Uuid::from_u128(104);
        let body = |section| {
            format!(
                "## Objective and scope\nA\n## Current decisions\nB\n## Outcome status{section}## Relevant evidence\n{}\n## Next actions and open questions\nEOF",
                history_link(id)
            )
        };
        for (original, change, expected) in [
            ("\nantes\n\n\n", "después", "\ndespués\n\n\n"),
            ("\r\nantes  \t\n \t\r\n\t\r", "después", "\r\ndespués\n \t\r\n\t\r"),
            ("\r\nantes\r", "después  ", "\r\ndespués  \r"),
            ("\r\nantes\r\n\r\n", "después", "\r\ndespués\r\n\r\n"),
            ("\r\nantes  \t\n \t\r\n\t\r", "", "\r\n \t\r\n\t\r"),
            ("\r\nantes\n", "", "\r\n"),
            ("\r\n \t\n\t\r", "después", "\r\ndespués\r\n \t\n\t\r"),
            ("\r\n \t\n\t\r", "", "\r\n \t\n\t\r"),
            ("\r\n", "después", "\r\ndespués\r\n"),
            ("\nantes\n\n\n", "después\n", "\ndespués\n"),
            ("\nantes\n\n\n", "después\n\n", "\ndespués\n\n"),
            ("\nantes\r\n", "después\r", "\ndespués\r"),
            ("\nantes\r\n\r\n", "después\r\n \t\r\n", "\ndespués\r\n \t\r\n"),
        ] {
            let doc = create_document(id, "Contexto", &body(original), None).unwrap();
            let changes = [("Outcome status".into(), change.into())].into();
            let next = doc.with_sections(&changes).unwrap();
            assert_eq!(next, body(expected), "{original:?} + {change:?}");
            let successor = doc.successor(&next).unwrap();
            assert_eq!(successor.with_sections(&changes).unwrap(), next);
        }
        let doc = create_document(id, "Contexto", &body("\rantes\n\n"), None).unwrap();
        for change in ["", "\ndespués"] {
            let changes = [("Outcome status".into(), change.into())].into();
            assert!(doc.with_sections(&changes).is_err(), "{change:?}");
        }
        let eof = [("Next actions and open questions".into(), "fin".into())].into();
        assert!(doc.with_sections(&eof).unwrap().ends_with("questions\nfin"));
    }
}

#[cfg(test)]
mod section_edit_tests {
    use super::*;

    #[test]
    fn canonical_heading_cannot_be_replaced_by_a_lookalike() {
        let id = Uuid::from_u128(101);
        let body = format!(
            "## Objective and scope\n\nObjetivo\n\n## Current decisions\n\n-->\n\n## Outcome status\n\nEstado\n\n## Relevant evidence\n\n{}\n\n## Next actions and open questions\n",
            history_link(id)
        );
        let doc = create_document(id, "Contexto", &body, None).unwrap();
        let replacement = "## Current decisions\n\n<!--";
        // Standalone validation succeeds: the comment hides the original H2
        // and the injected one now occupies its canonical spot in the order.
        assert!(doc.successor(&body.replace("Objetivo", replacement)).is_ok());
        let edit = SectionEdit {
            section: "Objective and scope".into(),
            old_text: "Objetivo".into(),
            new_text: replacement.into(),
        };
        assert!(doc.with_section_edits(&[edit]).is_err());
    }

    #[test]
    fn project_link_must_remain_active_but_need_not_be_unique() {
        let id = Uuid::from_u128(102);
        let project = ProjectAssociation {
            brain_project: Uuid::from_u128(103),
            path: "01-proyectos/Proyecto/Proyecto.md".into(),
            label: "Proyecto".into(),
        };
        let link = project.link();
        let edit = SectionEdit {
            section: "Current decisions".into(),
            old_text: link.clone(),
            new_text: format!("`{link}`"),
        };
        for other_active_link in [false, true] {
            let spare = if other_active_link { link.as_str() } else { "" };
            let body = format!(
                "## Objective and scope\n\n{spare}\n\n## Current decisions\n\n{link}\n\n## Outcome status\n\nEstado\n\n## Relevant evidence\n\n{}\n\n## Next actions and open questions\n",
                history_link(id)
            );
            let doc = create_document(id, "Contexto", &body, Some(&project)).unwrap();
            let candidate = doc.with_section_edits(std::slice::from_ref(&edit)).unwrap();
            assert_eq!(doc.successor(&candidate).is_ok(), other_active_link);
        }
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;
    fn document() -> MemoryDocument {
        let id = Uuid::from_u128(101);
        let body = format!("## Objective and scope\nObjetivo\n## Current decisions\nDecisión humana\n## Outcome status\nEstado\n## Relevant evidence\n{}\n## Next actions and open questions\nPróximos pasos\n", history_link(id));
        let doc = create_document(id, "Contexto", &body, None).unwrap();
        let raw = doc.raw.replace("memory_revision:", "# comentario humano\nunknown_field: conservar\nmemory_revision:").replace('\n', "\r\n");
        MemoryDocument::parse(id, &raw).unwrap()
    }
    fn project(n: u128) -> ProjectAssociation {
        ProjectAssociation { brain_project: Uuid::from_u128(n), path: format!("01-proyectos/P{n}/Proyecto.md"), label: format!("Proyecto {n}") }
    }
    #[test]
    fn association_preserves_identity_unknown_metadata_and_human_references() {
        let first = document();
        let next = first.with_project(&project(1)).unwrap();
        assert_eq!(next.revision, 2);
        assert_eq!(next.metadata.common().brain_id, first.metadata.common().brain_id);
        assert!(next.raw.contains("# comentario humano\r\nunknown_field: conservar\r\n"));
        assert!(next.body().contains("Decisión humana\r\n"));
        assert!(next.body().contains(&history_link(first.metadata.common().brain_id)));
        assert_eq!(next.with_project(&project(1)).unwrap().raw, next.raw);
        let rebound = next.with_project(&project(2)).unwrap();
        assert_eq!(rebound.revision, 3);
        assert_eq!(rebound.body().matches("<!-- brain:principal-project -->").count(), 1);
        assert!(rebound.body().contains(&project(2).link()));
        assert!(!rebound.body().contains(&project(1).link()));
    }
    #[test]
    fn literal_managed_marker_is_not_rewritten() {
        let first = document();
        let body = first.body().replace("Decisión humana", "```\n<!-- brain:principal-project -->\nEjemplo humano\n<!-- /brain:principal-project -->\n```");
        let literal = first.successor(&body).unwrap();
        assert!(literal.with_project(&project(1)).is_err());
    }
}
