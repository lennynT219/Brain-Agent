//! Lossless framing of accepted Markdown bytes. This module neither validates
//! YAML nor publishes files. Only a future verified publication layer may call
//! a snapshot accepted; frame parsing alone cannot establish that authority.
use anyhow::{Context, Result, bail};
use uuid::Uuid;

use super::current_link;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedSnapshot {
    pub revision: u64,
    pub request_id: Uuid,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub memory_id: Uuid,
    pub snapshots: Vec<AcceptedSnapshot>,
}

fn header(id: Uuid) -> String {
    format!(
        "# Historial\n\n<!-- brain-history-v1 {id} -->\n\n{}\n\n",
        current_link(id)
    )
}

fn fence_length(markdown: &str) -> Result<usize> {
    let mut longest = 0usize;
    let mut run = 0usize;
    for byte in markdown.bytes() {
        if byte == b'`' {
            run = run.checked_add(1).context("fence length overflow")?;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    Ok(longest
        .checked_add(1)
        .context("fence length overflow")?
        .max(3))
}

impl History {
    /// Empty history is a construction state, never an accepted baseline.
    pub fn new(memory_id: Uuid) -> Result<Self> {
        if memory_id.is_nil() {
            bail!("history requires a non-nil Memory identity");
        }
        Ok(Self {
            memory_id,
            snapshots: Vec::new(),
        })
    }

    /// Pure framing primitive. The caller must validate Markdown identity and
    /// human approval and confirm publication before reporting a saved state.
    pub fn push(&mut self, snapshot: AcceptedSnapshot) -> Result<()> {
        self.validate_sequence()?;
        let expected = self
            .latest()
            .map_or(Some(1), |last| last.revision.checked_add(1))
            .context("history revision overflow")?;
        if snapshot.revision != expected
            || snapshot.request_id.is_nil()
            || self
                .snapshots
                .iter()
                .any(|old| old.request_id == snapshot.request_id)
        {
            bail!("noncontiguous revision or reused/nil request identity");
        }
        self.snapshots
            .try_reserve(1)
            .context("history capacity unavailable")?;
        self.snapshots.push(snapshot);
        Ok(())
    }

    pub fn latest(&self) -> Option<&AcceptedSnapshot> {
        self.snapshots.last()
    }

    fn validate_sequence(&self) -> Result<()> {
        Self::new(self.memory_id)?;
        let mut seen = std::collections::HashSet::new();
        let mut previous = 0_u64;
        for snapshot in &self.snapshots {
            if Some(snapshot.revision) != previous.checked_add(1)
                || snapshot.request_id.is_nil()
                || !seen.insert(snapshot.request_id)
            {
                bail!("noncontiguous revision or reused/nil request identity");
            }
            previous = snapshot.revision;
        }
        Ok(())
    }

    /// Preserve snapshots exactly, including CRLF and absent final newline.
    /// Encoding never prunes, rotates or silently truncates accepted entries.
    pub fn encode(&self) -> Result<String> {
        if self.snapshots.is_empty() {
            bail!("empty history is not an accepted baseline");
        }
        // Public data can be assembled by callers; validate the sequence too.
        self.validate_sequence()?;
        let mut out = header(self.memory_id);
        for snapshot in &self.snapshots {
            let length = fence_length(&snapshot.markdown)?;
            let fence = "`".repeat(length);
            let prefix = format!(
                "## Revision {}\n<!-- snapshot {} {} {} -->\n{fence}markdown\n",
                snapshot.revision,
                snapshot.request_id,
                snapshot.markdown.len(),
                length,
            );
            let suffix = format!("\n{fence}\n<!-- end-snapshot -->\n\n");
            let required = prefix
                .len()
                .checked_add(snapshot.markdown.len())
                .and_then(|n| n.checked_add(suffix.len()))
                .context("history length overflow")?;
            out.try_reserve(required)
                .context("history capacity unavailable")?;
            out.push_str(&prefix);
            out.push_str(&snapshot.markdown);
            out.push_str(&suffix);
        }
        Ok(out)
    }

    pub fn parse(memory_id: Uuid, input: &str) -> Result<Self> {
        let mut history = Self::new(memory_id)?;
        let mut rest = input
            .strip_prefix(&header(memory_id))
            .context("invalid history header/identity")?;
        while !rest.is_empty() {
            let (heading, tail) = rest
                .split_once('\n')
                .context("incomplete revision heading")?;
            let revision = heading
                .strip_prefix("## Revision ")
                .context("invalid revision heading")?
                .parse::<u64>()
                .context("invalid history revision")?;
            let (metadata, tail) = tail
                .split_once('\n')
                .context("incomplete snapshot metadata")?;
            let fields = metadata
                .strip_prefix("<!-- snapshot ")
                .and_then(|s| s.strip_suffix(" -->"))
                .context("invalid snapshot metadata")?
                .split(' ')
                .collect::<Vec<_>>();
            if fields.len() != 3 {
                bail!("invalid snapshot metadata fields");
            }
            let request_id = Uuid::parse_str(fields[0]).context("invalid request identity")?;
            let byte_count = fields[1]
                .parse::<usize>()
                .context("invalid snapshot byte count")?;
            let length = fields[2]
                .parse::<usize>()
                .context("invalid snapshot fence length")?;
            // Never allocate an attacker-supplied fence length. It must fit the
            // actual opening line, then match the canonical content-safe fence.
            let (opening, tail) = tail
                .split_once('\n')
                .context("incomplete snapshot opening")?;
            let fence = opening
                .strip_suffix("markdown")
                .context("invalid snapshot language")?;
            if length < 3 || fence.len() != length || !fence.bytes().all(|b| b == b'`') {
                bail!("invalid snapshot fence");
            }
            let markdown = tail
                .get(..byte_count)
                .context("incomplete snapshot or non-UTF-8 boundary")?;
            let suffix = tail.get(byte_count..).context("invalid snapshot length")?;
            if fence_length(markdown)? != length {
                bail!("noncanonical or unsafe snapshot fence");
            }
            rest = suffix
                .strip_prefix('\n')
                .and_then(|s| s.strip_prefix(fence))
                .and_then(|s| s.strip_prefix("\n<!-- end-snapshot -->\n\n"))
                .context("incomplete snapshot footer")?;
            history.push(AcceptedSnapshot {
                revision,
                request_id,
                markdown: markdown.to_owned(),
            })?;
        }
        if history.snapshots.is_empty() {
            bail!("history has no accepted baseline");
        }
        Ok(history)
    }
}
