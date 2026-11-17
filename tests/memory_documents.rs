use brain::memory::{
    CURRENT_SECTIONS, history_link,
    markdown::{MemoryDocument, create_document},
};
use uuid::Uuid;

fn id() -> Uuid {
    Uuid::from_u128(1)
}
fn body() -> String {
    CURRENT_SECTIONS
        .iter()
        .map(|name| format!("## {name}\nContext\n\n"))
        .collect::<String>()
        + &history_link(id())
}

#[test]
fn real_yaml_reads_unknown_fields_comments_and_successor_preserves_them() {
    let original = create_document(id(), "Title: quoted", &body(), None).unwrap();
    let original = original
        .raw
        .replacen(
            "memory_revision: 1",
            "memory_revision: 1 # keep revision comment",
            1,
        )
        .replacen(
            "---\n",
            "---\nunknown:\n  nested: [one, two]\n# keep frontmatter comment\n",
            1,
        )
        .replace('\n', "\r\n");
    let parsed = MemoryDocument::parse(id(), &original).unwrap();
    let successor = parsed
        .successor(&parsed.body().replace("Context", "Updated"))
        .unwrap();
    assert!(
        successor
            .raw
            .contains("unknown:\r\n  nested: [one, two]\r\n# keep frontmatter comment\r\n")
    );
    assert!(
        successor
            .raw
            .contains("memory_revision: 2 # keep revision comment\r\n")
    );
    assert_eq!(successor.revision, 2);
}

#[test]
fn exactly_five_sections_are_enforced_outside_fences_and_comments() {
    let valid = create_document(id(), "Title", &body(), None).unwrap();
    for raw in [
        valid.raw.replace("## Current decisions", "## Other"),
        valid
            .raw
            .replace("## Current decisions", "## Outcome status"),
        format!("{}\n## Extra\n", valid.raw),
        format!("{}\nExtra Setext heading\n--\n", valid.raw),
        format!("{}\nExtra Setext heading\n---  \n", valid.raw),
        valid.raw.replace("brain_type: memory", "brain_type: note"),
        valid
            .raw
            .replace("brain_id:", "brain_id: invalid\nextra_id:"),
    ] {
        assert!(MemoryDocument::parse(id(), &raw).is_err());
    }
    assert!(MemoryDocument::parse(Uuid::from_u128(2), &valid.raw).is_err());
    let fenced=valid.body().replace("Context","<!--\n## Ignore comment\n-->\n~~~~markdown\n## Not a context section\n~~~\n~~~~\n    ## Indented code\n");
    assert!(valid.successor(&fenced).is_ok());
    assert!(
        valid
            .successor(&(valid.body().to_string() + "\n```markdown\nnot closed"))
            .is_err()
    );
}

#[test]
fn duplicate_yaml_and_invalid_revision_cannot_be_accepted() {
    let valid = create_document(id(), "Title", &body(), None).unwrap();
    let duplicate = valid.raw.replacen(
        "memory_revision: 1",
        "memory_revision: 1\nmemory_revision: 1",
        1,
    );
    assert!(MemoryDocument::parse(id(), &duplicate).is_err());
    let unsupported = valid
        .raw
        .replacen("memory_revision: 1", "'memory_revision': 1", 1);
    let parsed = MemoryDocument::parse(id(), &unsupported).unwrap();
    assert!(parsed.successor(parsed.body()).is_err());
    let unrelated = valid.raw.replace(&history_link(id()), "[[Historial]]");
    assert!(MemoryDocument::parse(id(), &unrelated).is_err());
    let overflow = valid.raw.replacen(
        "memory_revision: 1",
        "memory_revision: 18446744073709551615",
        1,
    );
    let parsed = MemoryDocument::parse(id(), &overflow).unwrap();
    assert!(parsed.successor(parsed.body()).is_err());
}
