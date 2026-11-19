//! Human-readable, append-only logical log. Exact accepted bytes are kept in
//! one replaceable sidecar, never in the dated entries.
use super::{current_link, markdown::MemoryDocument};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub(super) const HEAD: &str = ".accepted.json";

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub revision: u64,
    pub request: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Accepted {
    version: u32,
    memory_id: Uuid,
    revision: u64,
    request_id: Uuid,
    markdown: String,
}

fn header(id: Uuid) -> String {
    format!(
        "# Historial\n\n<!-- brain-change-log-v2 {id} -->\n\n{}\n\n",
        current_link(id)
    )
}

pub(super) fn is_log(raw: &str) -> bool {
    raw.starts_with("# Historial\n\n<!-- brain-change-log-v2 ")
}

pub(super) fn entries(id: Uuid, raw: &str) -> Result<Vec<Entry>> {
    ensure!(!id.is_nil(), "invalid log identity");
    let rest = raw
        .strip_prefix(&header(id))
        .context("invalid log header")?;
    let mut seen = HashSet::new();
    let mut entries: Vec<Entry> = Vec::new();
    for (chunk, suffix) in rest
        .split_inclusive("\n\n")
        .map(|s| (s, s.ends_with("\n\n")))
    {
        ensure!(suffix, "incomplete log entry");
        let [heading, marker, summary]: [&str; 3] = chunk
            .strip_suffix("\n\n")
            .unwrap()
            .split('\n')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid entry lines"))?;
        let date = heading
            .strip_prefix("## ")
            .context("invalid date heading")?;
        ensure!(
            date.len() == 10
                && date
                    .bytes()
                    .enumerate()
                    .all(|(i, c)| if matches!(i, 4 | 7) {
                        c == b'-'
                    } else {
                        c.is_ascii_digit()
                    }),
            "invalid entry date"
        );
        let (revision, request) = marker
            .strip_prefix("<!-- change ")
            .and_then(|s| s.strip_suffix(" -->"))
            .and_then(|s| s.split_once(' '))
            .context("invalid change marker")?;
        let revision: u64 = revision.parse()?;
        let request = Uuid::parse_str(request)?;
        let summary = summary
            .strip_prefix("- ")
            .context("invalid entry summary")?;
        ensure!(
            !summary.trim().is_empty()
                && summary.len() <= 2048
                && !summary.chars().any(char::is_control),
            "invalid entry summary"
        );
        ensure!(
            revision > 0
                && !request.is_nil()
                && seen.insert(request)
                && entries
                    .last()
                    .is_none_or(|e| e.revision.checked_add(1) == Some(revision)),
            "invalid log sequence"
        );
        entries.push(Entry { revision, request });
    }
    ensure!(!entries.is_empty(), "missing log baseline");
    Ok(entries)
}

fn utc_date() -> Result<String> {
    let mut day = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 86_400;
    let mut year = 1970;
    let leap = |y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    while year <= 9999 {
        let days = 365 + u64::from(leap(year));
        if day < days {
            break;
        }
        day -= days;
        year += 1;
    }
    ensure!(year <= 9999, "date outside supported range");
    for (i, days) in [
        31,
        28 + u64::from(leap(year)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
    .into_iter()
    .enumerate()
    {
        if day < days {
            return Ok(format!("{year:04}-{:02}-{:02}", i + 1, day + 1));
        }
        day -= days;
    }
    bail!("invalid date")
}

pub(super) fn validate_summary(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control),
        "provide a one-line summary up to 1024 bytes"
    );
    Ok(())
}

pub(super) fn append(
    id: Uuid,
    previous: Option<&str>,
    revision: u64,
    request: Uuid,
    summary: &str,
) -> Result<String> {
    validate_summary(summary)?;
    if let Some(previous) = previous {
        let old = entries(id, previous)?;
        ensure!(
            old.last().and_then(|e| e.revision.checked_add(1)) == Some(revision)
                && !old.iter().any(|e| e.request == request),
            "invalid successor entry"
        );
    }
    ensure!(
        revision > 0 && !id.is_nil() && !request.is_nil(),
        "invalid entry identity"
    );
    let mut output = previous.map_or_else(|| header(id), str::to_owned);
    let summary = summary
        .trim()
        .chars()
        .flat_map(|c| {
            (c.is_ascii_punctuation()
                && !matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | '(' | ')' | '-'))
            .then_some('\\')
            .into_iter()
            .chain(std::iter::once(c))
        })
        .collect::<String>();
    output.push_str(&format!(
        "## {}\n<!-- change {revision} {request} -->\n- {summary}\n\n",
        utc_date()?
    ));
    Ok(output)
}

pub(super) fn encode_head(id: Uuid, request: Uuid, current: &MemoryDocument) -> Result<String> {
    Ok(serde_json::to_string(&Accepted {
        version: 2,
        memory_id: id,
        revision: current.revision,
        request_id: request,
        markdown: current.raw.clone(),
    })?)
}

pub(super) fn baseline(id: Uuid, log: &str, head: &str) -> Result<MemoryDocument> {
    let entries = entries(id, log)?;
    let last = entries.last().context("missing tip")?;
    let accepted: Accepted = serde_json::from_str(head)?;
    ensure!(
        accepted.version == 2
            && accepted.memory_id == id
            && accepted.revision == last.revision
            && accepted.request_id == last.request,
        "log and accepted baseline disagree"
    );
    let doc = MemoryDocument::parse(id, &accepted.markdown)?;
    ensure!(
        doc.revision == accepted.revision,
        "invalid accepted revision"
    );
    Ok(doc)
}
