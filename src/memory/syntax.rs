//! Markdown topology comes from pulldown-cmark, not prefix/indent heuristics.
//! Only validation views/events are interpreted; original document bytes remain
//! authoritative. No HTML rendering, link following or source reserialization.
use super::CURRENT_SECTIONS;
use anyhow::{Result, bail};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::{borrow::Cow, ops::Range};

/// Fixed diagnostic category; never carries document content or paths.
#[derive(Debug)]
pub(crate) struct UnclosedHtml;

impl std::fmt::Display for UnclosedHtml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unclosed literal HTML element/comment")
    }
}
impl std::error::Error for UnclosedHtml {}

pub(crate) fn active_relations(body: &str) -> Result<Vec<String>> {
    // pulldown's fenced-info handling does not consistently split bare CR.
    // An equal-length view makes CR a logical newline while retaining an exact
    // byte-for-byte source map, including original CRLF and all stored bytes.
    let lines = newline_view(body);
    let view = comment_view(&lines)?;
    let mut stack = Vec::new();
    let mut headings = 0usize;
    let mut links = Vec::new();
    let mut html: Option<(String, usize)> = None;
    let mut code: Option<(Option<(u8, usize)>, usize)> = None;
    for (event, range) in Parser::new_ext(&view, Options::ENABLE_WIKILINKS).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                match &tag {
                    Tag::Heading {
                        level: HeadingLevel::H2,
                        ..
                    } if stack.is_empty() && html.is_none() => {
                        let line = body[range.clone()].split(['\r', '\n']).next().unwrap_or("");
                        let at_line_start = range.start == 0
                            || matches!(body.as_bytes()[range.start - 1], b'\r' | b'\n');
                        if !at_line_start
                            || CURRENT_SECTIONS
                                .get(headings)
                                .is_none_or(|name| line != format!("## {name}"))
                        {
                            bail!("Memory requires exactly five ordered canonical top-level H2s");
                        }
                        headings += 1;
                    }
                    Tag::CodeBlock(kind) => {
                        let fence = if matches!(kind, CodeBlockKind::Fenced(_)) {
                            let marker = view.as_bytes()[range.start];
                            let count = view.as_bytes()[range.start..]
                                .iter()
                                .take_while(|b| **b == marker)
                                .count();
                            Some((marker, count))
                        } else {
                            None
                        };
                        let opening_end = view[range.clone()]
                            .find('\n')
                            .map_or(range.end, |n| range.start + n + 1);
                        code = Some((fence, opening_end));
                    }
                    Tag::Link {
                        link_type: LinkType::WikiLink { .. },
                        ..
                    } if html.is_none()
                        && code.is_none()
                        && !stack
                            .iter()
                            .any(|tag| matches!(tag, TagEnd::Image | TagEnd::Link)) =>
                    {
                        if let Some(token) = original_wikilink(body, range.clone()) {
                            links.push(token.to_owned());
                        }
                    }
                    _ => {}
                }
                stack.push(tag.to_end());
            }
            Event::End(tag) => {
                if tag == TagEnd::CodeBlock {
                    if let Some((Some((marker, count)), content_end)) = code {
                        // The parser's code Text ranges exclude its closing
                        // fence (and quote/list prefixes). Validate closure only
                        // in that excluded tail, never by scanning code content.
                        let tail = view[content_end.min(range.end)..range.end]
                            .trim_end_matches([' ', '\t', '\r', '\n']);
                        if tail.bytes().rev().take_while(|b| *b == marker).count() < count {
                            bail!("unclosed fenced code block");
                        }
                    }
                    code = None;
                }
                if stack.pop() != Some(tag) {
                    bail!("unbalanced Markdown parser events");
                }
            }
            Event::Text(_) => {
                if let Some((_, content_end)) = code.as_mut() {
                    *content_end = range.end;
                } else if !stack
                    .iter()
                    .any(|tag| matches!(tag, TagEnd::Link | TagEnd::Image))
                {
                    // Preserve the existing fail-closed malformed-HTML policy.
                    // A malformed tag is plain Text to CommonMark; do not let a
                    // wikilink-shaped attribute in it become an active relation.
                    for (offset, _) in view[range.clone()].match_indices('<') {
                        let start = range.start + offset;
                        if looks_like_html(&view.as_bytes()[start..]) && !escaped(&view, start) {
                            validate_tag(&view[start..])?;
                        }
                    }
                }
            }
            Event::InlineHtml(_) | Event::Html(_) => {
                html_literals(&view[range], &mut html)?;
            }
            Event::Rule if body[range].trim().bytes().all(|b| b == b'-') => {
                bail!("dash-only dividers unsupported; use ***");
            }
            _ => {}
        }
    }
    if html.is_some() {
        return Err(UnclosedHtml.into());
    }
    if headings != CURRENT_SECTIONS.len() {
        bail!("Memory requires exactly five ordered canonical top-level H2s");
    }
    Ok(links)
}

/// Parser-recognized HTML comments are literal through their source-level
/// terminator, even when ending a quote/list makes the parser emit ordinary
/// Heading/Link/Text events in between. Mask them BEFORE the shared topology
/// pass, retaining every byte position and newline. This also handles closure
/// in Text/Code/attributes, not only another Html event. Attributes, escaped
/// openers and code are not independently rescanned for comment openers.
fn comment_view(source: &str) -> Result<Cow<'_, str>> {
    let mut view = Cow::Borrowed(source);
    // Resolve ONLY the earliest currently recognized comment, then reparse.
    // Its opaque contents can otherwise hide a later real comment as code, or
    // falsely expose a comment-shaped string in later legitimate code.
    // Progress is monotonic: each round removes a previously visible "<!--",
    // never creates one, preserves byte positions/newlines and has no pass cap.
    while let Some(span) = next_comment(&view)? {
        let mut masked = view.as_bytes().to_vec();
        for byte in &mut masked[span] {
            if !matches!(*byte, b'\r' | b'\n') {
                *byte = b' ';
            }
        }
        view = Cow::Owned(String::from_utf8(masked).expect("whole comment replaced by ASCII"));
    }
    Ok(view)
}

fn next_comment(source: &str) -> Result<Option<Range<usize>>> {
    for (event, range) in Parser::new_ext(source, Options::ENABLE_WIKILINKS).into_offset_iter() {
        if !matches!(event, Event::Html(_) | Event::InlineHtml(_)) {
            continue;
        }
        let mut offset = range.start;
        while offset < range.end {
            let Some(next) = source[offset..range.end].find('<') else {
                break;
            };
            offset += next;
            if source[offset..].starts_with("<!--") {
                let Some(end) = source[offset + 4..].find("-->") else {
                    return Err(UnclosedHtml.into());
                };
                return Ok(Some(offset..offset + 4 + end + 3));
            } else if looks_like_html(&source.as_bytes()[offset..]) {
                let (length, _, _, _) = validate_tag(&source[offset..])?;
                offset += length; // Never mistake an attribute value for an opener.
            } else {
                offset += 1;
            }
        }
    }
    Ok(None)
}

fn newline_view(body: &str) -> Cow<'_, str> {
    if body
        .as_bytes()
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && body.as_bytes().get(i + 1) != Some(&b'\n'))
    {
        let mut bytes = body.as_bytes().to_vec();
        for (i, b) in body.bytes().enumerate() {
            if b == b'\r' && body.as_bytes().get(i + 1) != Some(&b'\n') {
                bytes[i] = b'\n';
            }
        }
        Cow::Owned(String::from_utf8(bytes).expect("only ASCII CR replaced"))
    } else {
        Cow::Borrowed(body)
    }
}
fn original_wikilink(body: &str, range: Range<usize>) -> Option<&str> {
    let token = body.get(range.clone())?;
    let inner = token.strip_prefix("[[")?.strip_suffix("]]")?;
    if inner.contains(['[', ']', '\\', '\r', '\n'])
        || range
            .start
            .checked_sub(1)
            .is_some_and(|i| body.as_bytes()[i] == b'[')
        || body.as_bytes().get(range.end) == Some(&b']')
    {
        return None;
    }
    Some(token)
}
fn escaped(text: &str, offset: usize) -> bool {
    text.as_bytes()[..offset]
        .iter()
        .rev()
        .take_while(|b| **b == b'\\')
        .count()
        % 2
        == 1
}
fn looks_like_html(bytes: &[u8]) -> bool {
    bytes
        .get(1)
        .is_some_and(|b| b.is_ascii_alphabetic() || matches!(*b, b'/' | b'!' | b'?'))
}
fn html_literals(raw: &str, html: &mut Option<(String, usize)>) -> Result<()> {
    let mut offset = 0;
    while offset < raw.len() {
        let Some(next) = raw[offset..].find('<') else {
            break;
        };
        offset += next;
        if !looks_like_html(&raw.as_bytes()[offset..]) {
            offset += 1;
            continue;
        }
        let (length, tag, closing, standalone) = validate_tag(&raw[offset..])?;
        if let Some((name, depth)) = html.as_mut() {
            if tag.as_deref() == Some(name.as_str()) {
                if closing {
                    *depth -= 1;
                } else if !standalone {
                    *depth += 1;
                }
                if *depth == 0 {
                    *html = None;
                }
            }
        } else if !closing
            && !standalone
            && let Some(name) = tag
        {
            *html = Some((name, 1));
        }
        offset += length;
    }
    Ok(())
}
fn validate_tag(raw: &str) -> Result<(usize, Option<String>, bool, bool)> {
    let mut quote = None;
    let mut end = None;
    for (i, byte) in raw.bytes().enumerate().skip(1) {
        if matches!(byte, b'\r' | b'\n') {
            bail!("multiline or unclosed HTML tag syntax is unsupported");
        }
        if let Some(q) = quote {
            if byte == q {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'>' {
            end = Some(i);
            break;
        }
    }
    let Some(end) = end else {
        bail!("multiline or unclosed HTML tag syntax is unsupported");
    };
    let mut content = raw[1..end].trim();
    let closing = content.starts_with('/');
    if closing {
        content = &content[1..];
    }
    let length = content
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
        .count();
    let tag = if length != 0
        && content.as_bytes()[0].is_ascii_alphabetic()
        && content
            .as_bytes()
            .get(length)
            .is_none_or(|b| b.is_ascii_whitespace() || *b == b'/')
    {
        Some(content[..length].to_ascii_lowercase())
    } else {
        None
    };
    let standalone = content.ends_with('/')
        || tag.as_deref().is_none_or(|tag| {
            matches!(
                tag,
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            )
        });
    Ok((end + 1, tag, closing, standalone))
}
