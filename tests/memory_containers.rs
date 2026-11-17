//! Remaining R1 container regressions; every vault/work marker is synthetic.
use brain::memory::{
    CURRENT_SECTIONS,
    history::{AcceptedSnapshot, History},
    history_link,
    markdown::{MemoryDocument, ProjectAssociation},
    store::{Context, Create, Expected, Store},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn containers(link: &str) -> Vec<String> {
    vec![
        format!("> ~~~text\n> {link}\n> ~~~"),
        format!("> !{link}"),
        format!("- !{link}"),
        format!("> \\{link}"),
        format!("- `{link}`"),
        format!("> <!-- {link} -->"),
        format!("- [example](<foo){link}>)"),
        format!("> <span title='{link}'>literal</span>"),
        format!("> [ref]: {link}"),
        format!("example\n> ~~~text\n> {link}\n> ~~~"),
        format!("example\n>     {link}"),
        format!("example\n- ~~~text\n  {link}\n  ~~~"),
        format!("> ```text\n> {link}\n> ```"),
        format!(">     {link}"),
        format!(">\t\t{link}"),
        format!("   >     {link}"),
        format!(">> ~~~text\n>> {link}\n>> ~~~"),
        format!("> >     {link}"),
        format!("> - ~~~text\n>   {link}\n>   ~~~"),
        format!("- > ~~~text\n  > {link}\n  > ~~~"),
        format!("- ~~~text\n  {link}\n  ~~~"),
        format!("+ item\n\n      {link}"),
        format!("* item\n\n      {link}"),
        format!("1. ~~~text\n   {link}\n   ~~~"),
        format!("123456789) >     {link}"),
    ]
}
fn path(vault: &Path, memory: Uuid, name: &str) -> PathBuf {
    vault
        .join("05-memorias")
        .join(memory.to_string())
        .join(name)
}
fn project() -> ProjectAssociation {
    ProjectAssociation {
        brain_project: id(20),
        path: "01-proyectos/P.md".into(),
        label: "P".into(),
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Store, Uuid, Expected) {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    for dir in [
        "00-limbo",
        "01-proyectos",
        "02-areas",
        "03-recursos",
        "04-archivo",
        "99-sistema/plantillas",
        "99-sistema/documentacion",
    ] {
        fs::create_dir_all(vault.join(dir)).unwrap();
    }
    fs::write(
        vault.join("01-proyectos/P.md"),
        format!(
            "---\nbrain_type: project\nbrain_version: 1\nbrain_id: {}\ntitle: P\n---\n",
            id(20)
        ),
    )
    .unwrap();
    let work = temp.path().join("work");
    fs::create_dir(&work).unwrap();
    let store = Store::open(&vault).unwrap();
    let memory = store
        .create(
            id(1),
            Create {
                context: Context {
                    working_directory: work.clone(),
                    authorized_boundary: work,
                },
                title: "Container review".into(),
                sections: std::array::from_fn(|_| "Evidence".into()),
                project: Some(project()),
                memory_subfolders: vec![],
            },
        )
        .unwrap()
        .memory_id;
    // Seed an exact, valid accepted fixture with unknown YAML, CRLF and no final
    // newline. This is test setup, not a production import/repair operation.
    let raw = store
        .read(memory)
        .unwrap()
        .accepted_markdown
        .unwrap()
        .replacen("title:", "unknown: {custom: [1, 2]} # preserve\ntitle:", 1)
        .replace('\n', "\r\n")
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    MemoryDocument::parse(memory, &raw).unwrap();
    seed(&vault, memory, &raw);
    assert_eq!(store.read(memory).unwrap().status, "found");
    (
        temp,
        vault,
        store,
        memory,
        Expected {
            revision: 1,
            accepted_markdown: raw,
        },
    )
}
fn seed(vault: &Path, memory: Uuid, raw: &str) {
    let mut history = History::new(memory).unwrap();
    history
        .push(AcceptedSnapshot {
            revision: 1,
            request_id: id(1),
            markdown: raw.into(),
        })
        .unwrap();
    fs::write(path(vault, memory, "Memoria.md"), raw).unwrap();
    fs::write(
        path(vault, memory, "Historial.md"),
        history.encode().unwrap(),
    )
    .unwrap();
}
fn cli(root: &Path, request: serde_json::Value) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_brain"))
        .arg("memory")
        .current_dir(root)
        .env("HOME", root.join("unused-home"))
        .env("USERPROFILE", root.join("unused-home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn quoted_and_nested_container_relations_cannot_save_history_or_project_revision() {
    let (temp, vault, store, memory, expected) = fixture();
    let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
    let before = (
        fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
        fs::read(path(&vault, memory, "Historial.md")).unwrap(),
    );
    for link in [history_link(memory), project().link()] {
        for wrapped in containers(&link) {
            for newline in ["\n", "\r\n", "\r"] {
                let body = doc.body().replace(&link, &wrapped.replace('\n', newline));
                let result = cli(
                    temp.path(),
                    serde_json::json!({"protocol_version":1,"request_id":id(2),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
                );
                assert_eq!(
                    result["result"]["status"], "invalid",
                    "saved container relation: {wrapped}"
                );
                assert_eq!(
                    fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                    before.0
                );
                assert_eq!(
                    fs::read(path(&vault, memory, "Historial.md")).unwrap(),
                    before.1
                );
                assert!(!path(&vault, memory, ".pending.json").exists());
                assert_eq!(store.read(memory).unwrap().revision, Some(1));
            }
        }
    }
}

#[test]
fn ordinary_containers_and_inactive_examples_allow_a_separate_active_relation() {
    let (_temp, _vault, _store, memory, expected) = fixture();
    let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
    for link in [history_link(memory), project().link()] {
        for wrapped in containers(&link).into_iter().chain([
            format!("> {link}"),
            format!("- {link}"),
            format!("1) {link}"),
        ]) {
            assert!(
                doc.successor(&format!("{}\r\n\r\n{wrapped}", doc.body()))
                    .is_ok()
            );
        }
    }
    // Only the five canonical top-level H2s count, not quoted headings.
    for name in CURRENT_SECTIONS {
        let body = doc
            .body()
            .replace(&format!("## {name}"), &format!("> ## {name}"));
        assert!(doc.successor(&body).is_err());
    }
}

#[test]
fn active_relations_with_supported_literal_examples_preserve_raw_accepted_bytes() {
    let (_temp, vault, store, memory, expected) = fixture();
    let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
    let mut body = doc.body().replace("Evidence", "Updated evidence");
    for link in [history_link(memory), project().link()] {
        for newline in ["\n", "\r\n", "\r"] {
            body.push_str(&format!("\n\n````text\n> ~~~text\n> {link}\n> ~~~\n- >     {link}\n````\n\n<!--\n>     {link}\n-->\n\n<div>\n>     {link}\n</div>\n\n    >     {link}\n\n`{link}`\n").replace('\n',newline));
        }
    }
    // The real links remain unquoted in the original five canonical sections.
    let wanted = doc.successor(&body).unwrap().raw;
    let saved = store
        .update(memory, id(2), expected.clone(), &body)
        .unwrap();
    assert_eq!(saved.revision, 2);
    let current = fs::read_to_string(path(&vault, memory, "Memoria.md")).unwrap();
    assert_eq!(current, wanted);
    assert!(current.contains("unknown: {custom: [1, 2]} # preserve\r\n"));
    assert!(current.starts_with(
        &expected.accepted_markdown[..doc.raw.len() - doc.body().len()].replacen(
            "memory_revision: 1",
            "memory_revision: 2",
            1
        )
    ));
    let history = History::parse(
        memory,
        &fs::read_to_string(path(&vault, memory, "Historial.md")).unwrap(),
    )
    .unwrap();
    assert_eq!(history.snapshots.len(), 2);
    assert_eq!(history.snapshots[0].markdown, expected.accepted_markdown);
    assert_eq!(history.snapshots[1].markdown, current);
}

fn active_forms(link: &str) -> Vec<String> {
    vec![
        format!("- {link}"),
        format!("1. {link}"),
        format!("- [ ] {link}"),
        format!("- [x] {link}"),
        format!("> {link}"),
        format!("> > {link}"),
        format!("- > {link}"),
        format!("> - {link}"),
        format!("- Explanation\n  {link}"),
        format!("Paragraph continuation\n    {link}"),
        format!("> Explanation\n{link}"),
        format!("1. Explanation\n   - {link}"),
    ]
}

#[test]
fn genuine_history_and_project_links_inside_containers_save_exact_successors() {
    let (temp, vault, store, memory, mut expected) = fixture();
    let original = MemoryDocument::parse(memory, &expected.accepted_markdown)
        .unwrap()
        .body()
        .replace("\r\n", "\n");
    for link in [history_link(memory), project().link()] {
        for form in active_forms(&link) {
            for newline in ["\n", "\r\n", "\r"] {
                let body = original.replace(&link, &form).replace('\n', newline);
                let old = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
                let next_revision = expected.revision + 1;
                let prefix = old.raw[..old.raw.len() - old.body().len()].replacen(
                    &format!("memory_revision: {}", expected.revision),
                    &format!("memory_revision: {next_revision}"),
                    1,
                );
                let wanted = format!("{prefix}{body}");
                let result = cli(
                    temp.path(),
                    serde_json::json!({"protocol_version":1,"request_id":id(2+next_revision as u128),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
                );
                assert_eq!(
                    result["result"]["status"], "saved",
                    "active container {form:?}, newline {newline:?}"
                );
                assert_eq!(result["result"]["revision"], next_revision);
                let read = store.read(memory).unwrap();
                assert_eq!(read.status, "found");
                assert_eq!(read.accepted_markdown.as_deref(), Some(wanted.as_str()));
                assert_eq!(
                    fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                    wanted.as_bytes()
                );
                let history = History::parse(
                    memory,
                    &fs::read_to_string(path(&vault, memory, "Historial.md")).unwrap(),
                )
                .unwrap();
                assert_eq!(history.latest().unwrap().markdown, wanted);
                expected = Expected {
                    revision: next_revision,
                    accepted_markdown: wanted,
                };
            }
        }
    }
}

#[test]
fn old_valid_list_and_quote_snapshots_remain_found_byte_exact() {
    for text in [
        "- Keep context\n- Preserve accepted history",
        "1. Keep context\n2. Preserve accepted history",
        "> Ordinary explanatory quotation",
        "- [ ] Markdown text, not a Brain Task",
        "> - A nested list\n>   with lazy continuation",
    ] {
        let (temp, vault, store, memory, expected) = fixture();
        let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
        let body = doc
            .body()
            .replacen("Evidence", &text.replace('\n', "\r\n"), 1);
        let raw = format!("{}{}", &doc.raw[..doc.raw.len() - doc.body().len()], body);
        seed(&vault, memory, &raw);
        let marker = temp.path().join("work/.brain.toml");
        let before = (
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
            fs::read(&marker).unwrap(),
        );
        let read = store.read(memory).unwrap();
        assert_eq!(read.status, "found", "previously valid {text}");
        assert_eq!(read.revision, Some(1));
        assert_eq!(read.accepted_markdown.as_deref(), Some(raw.as_str()));
        assert_eq!(
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            before.0
        );
        assert_eq!(
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
            before.1
        );
        assert_eq!(fs::read(&marker).unwrap(), before.2);
    }
}

fn carried_comment(link: &str, opener: &str, ending: &str) -> String {
    format!("{opener}\n\nLiteral é\n{link}\n\n{ending}")
}

#[test]
fn comment_context_crosses_events_for_both_relations_and_closes_in_any_leaf() {
    let (temp, vault, store, memory, mut expected) = fixture();
    let original = MemoryDocument::parse(memory, &expected.accepted_markdown)
        .unwrap()
        .body()
        .replace("\r\n", "\n");
    for link in [history_link(memory), project().link()] {
        for opener in ["> <!--", "- <!--"] {
            for ending in [
                "<span data-end=\"-->\"></span>",
                "<!-- -->",
                "plain text --> tail",
                "`-->`",
            ] {
                for newline in ["\n", "\r\n", "\r"] {
                    let wrapped = carried_comment(&link, opener, ending);
                    let before = (
                        fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                        fs::read(path(&vault, memory, "Historial.md")).unwrap(),
                        fs::read(temp.path().join("work/.brain.toml")).unwrap(),
                    );
                    for active in [false, true] {
                        let replacement = if active {
                            format!("{wrapped}\n\n{link}")
                        } else {
                            wrapped.clone()
                        };
                        let body = original.replace(&link, &replacement).replace('\n', newline);
                        let response = cli(
                            temp.path(),
                            serde_json::json!({"protocol_version":1,"request_id":id(1000+expected.revision as u128),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
                        );
                        assert_eq!(
                            response["result"]["status"],
                            if active { "saved" } else { "invalid" },
                            "{opener:?}, {ending:?}, {newline:?}, active={active}"
                        );
                        if active {
                            let old =
                                MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
                            let revision = expected.revision + 1;
                            let prefix = old.raw[..old.raw.len() - old.body().len()].replacen(
                                &format!("memory_revision: {}", expected.revision),
                                &format!("memory_revision: {revision}"),
                                1,
                            );
                            let wanted = format!("{prefix}{body}");
                            let read = store.read(memory).unwrap();
                            assert_eq!(read.status, "found");
                            assert_eq!(read.revision, Some(revision));
                            assert_eq!(read.accepted_markdown.as_deref(), Some(wanted.as_str()));
                            let history = History::parse(
                                memory,
                                &fs::read_to_string(path(&vault, memory, "Historial.md")).unwrap(),
                            )
                            .unwrap();
                            assert_eq!(history.latest().unwrap().markdown, wanted);
                            expected = Expected {
                                revision,
                                accepted_markdown: wanted,
                            };
                        } else {
                            assert_eq!(
                                store.read(memory).unwrap().revision,
                                Some(expected.revision)
                            );
                            assert_eq!(
                                fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                                before.0
                            );
                            assert_eq!(
                                fs::read(path(&vault, memory, "Historial.md")).unwrap(),
                                before.1
                            );
                        }
                        assert_eq!(
                            fs::read(temp.path().join("work/.brain.toml")).unwrap(),
                            before.2
                        );
                        assert!(!path(&vault, memory, ".pending.json").exists());
                    }
                }
            }
        }
    }
}

#[test]
fn carried_comment_hides_literal_headings_but_keeps_real_sections_after_closure() {
    for opener in ["> <!--", "- <!--"] {
        for newline in ["\n", "\r\n", "\r"] {
            let (temp, vault, store, memory, expected) = fixture();
            let real = MemoryDocument::parse(memory, &expected.accepted_markdown)
                .unwrap()
                .body()
                .replace("\r\n", "\n");
            let hidden = CURRENT_SECTIONS
                .iter()
                .map(|name| format!("## {name}\n\n"))
                .collect::<String>();
            let body = format!(
                "{opener}\n\n{hidden}<span data-end=\"-->\"></span>\n\n{}\n\n{}",
                history_link(memory),
                project().link()
            )
            .replace('\n', newline);
            let before = (
                fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                fs::read(path(&vault, memory, "Historial.md")).unwrap(),
            );
            let response = cli(
                temp.path(),
                serde_json::json!({"protocol_version":1,"request_id":id(3),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
            );
            assert_eq!(response["result"]["status"], "invalid");
            assert_eq!(
                fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                before.0
            );
            assert_eq!(
                fs::read(path(&vault, memory, "Historial.md")).unwrap(),
                before.1
            );
            let body=format!("{opener}\n\n## Not a real H2\n\n```ignored fence\n<span data-end=\"-->\"></span>\n\n{real}").replace('\n',newline);
            let response = cli(
                temp.path(),
                serde_json::json!({"protocol_version":1,"request_id":id(4),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
            );
            assert_eq!(response["result"]["status"], "saved");
            assert_eq!(store.read(memory).unwrap().revision, Some(2));
            let doc = MemoryDocument::parse(
                memory,
                &store.read(memory).unwrap().accepted_markdown.unwrap(),
            )
            .unwrap();
            assert_eq!(doc.body(), body);
        }
    }
}

fn opaque_comment(opener: &str, fence: &str) -> String {
    format!("{opener}\n\n{fence}literal é\n## Not a real H2\n-->\n\n")
}

#[test]
fn ordered_comment_rediscovery_preserves_chains_and_neighboring_real_code() {
    let (temp, vault, store, memory, mut expected) = fixture();
    let original = MemoryDocument::parse(memory, &expected.accepted_markdown)
        .unwrap()
        .body()
        .replace("\r\n", "\n");
    for opener in ["> <!--", "- <!--"] {
        for fence in ["```", "~~~"] {
            let first = opaque_comment(opener, fence);
            let suffixes = [
                "<!--\nsecond ordinary comment\n-->\n\n".to_owned(),
                format!("{fence}actual code\n<!-- not a comment\n{fence}\n\n"),
                "`<!-- not a comment`\n\n".to_owned(),
                "\\<!-- not a comment\n\n".to_owned(),
                "<span data-marker=\"<!-- not a comment\">ordinary</span>\n\n".to_owned(),
            ];
            let mut prefixes = suffixes
                .into_iter()
                .map(|suffix| format!("{first}{suffix}"))
                .collect::<Vec<_>>();
            prefixes.push(format!(
                "{}<!--\nlast real comment\n-->\n\n",
                first.repeat(4)
            ));
            for prefix in prefixes {
                for newline in ["\n", "\r\n", "\r"] {
                    let body = format!("{prefix}{original}").replace('\n', newline);
                    let old = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
                    let revision = expected.revision + 1;
                    let prefix = old.raw[..old.raw.len() - old.body().len()].replacen(
                        &format!("memory_revision: {}", expected.revision),
                        &format!("memory_revision: {revision}"),
                        1,
                    );
                    let wanted = format!("{prefix}{body}");
                    let response = cli(
                        temp.path(),
                        serde_json::json!({"protocol_version":1,"request_id":id(4000+revision as u128),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
                    );
                    assert_eq!(
                        response["result"]["status"], "saved",
                        "{opener:?} {fence:?} {newline:?} {body:?}"
                    );
                    let read = store.read(memory).unwrap();
                    assert_eq!(read.status, "found");
                    assert_eq!(read.revision, Some(revision));
                    assert_eq!(read.accepted_markdown.as_deref(), Some(wanted.as_str()));
                    assert_eq!(
                        fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                        wanted.as_bytes()
                    );
                    let history = History::parse(
                        memory,
                        &fs::read_to_string(path(&vault, memory, "Historial.md")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(history.latest().unwrap().markdown, wanted);
                    expected = Expected {
                        revision,
                        accepted_markdown: wanted,
                    };
                }
            }
        }
    }
}

#[test]
fn rediscovered_later_comments_cannot_supply_either_required_relation() {
    let (temp, vault, store, memory, expected) = fixture();
    let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
    let original = doc.body().replace("\r\n", "\n");
    let before = (
        fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
        fs::read(path(&vault, memory, "Historial.md")).unwrap(),
        fs::read(temp.path().join("work/.brain.toml")).unwrap(),
    );
    for link in [history_link(memory), project().link()] {
        for opener in ["> <!--", "- <!--"] {
            for newline in ["\n", "\r\n", "\r"] {
                let comments = format!(
                    "{}<!--\n{link}\n## Still literal\n-->\n",
                    opaque_comment(opener, "```")
                );
                let body = original.replace(&link, &comments).replace('\n', newline);
                let response = cli(
                    temp.path(),
                    serde_json::json!({"protocol_version":1,"request_id":id(5000),"operation":"update","vault":vault,"arguments":{"memory_id":memory,"expected":expected,"body":body}}),
                );
                assert_eq!(response["result"]["status"], "invalid");
                assert_eq!(store.read(memory).unwrap().revision, Some(1));
                assert_eq!(
                    fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
                    before.0
                );
                assert_eq!(
                    fs::read(path(&vault, memory, "Historial.md")).unwrap(),
                    before.1
                );
                assert_eq!(
                    fs::read(temp.path().join("work/.brain.toml")).unwrap(),
                    before.2
                );
                assert!(!path(&vault, memory, ".pending.json").exists());
            }
        }
    }
}

#[test]
fn previously_accepted_comment_only_relations_are_partial_and_preserved() {
    for relation in ["history", "project"] {
        let (_temp, vault, store, memory, expected) = fixture();
        let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
        let link = if relation == "history" {
            history_link(memory)
        } else {
            project().link()
        };
        let body = doc.body().replace(
            &link,
            &carried_comment(&link, "> <!--", "<span data-end=\"-->\"></span>"),
        );
        let raw = format!("{}{}", &doc.raw[..doc.raw.len() - doc.body().len()], body);
        seed(&vault, memory, &raw);
        let before = (
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
        );
        let read = store.read(memory).unwrap();
        assert_eq!(read.status, "partial");
        assert_eq!(read.reason.as_deref(), Some("history_document_invalid"));
        assert_eq!(
            store
                .update(
                    memory,
                    id(2),
                    Expected {
                        revision: 1,
                        accepted_markdown: raw
                    },
                    "ignored"
                )
                .unwrap_err()
                .status,
            "partial"
        );
        assert_eq!(
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            before.0
        );
        assert_eq!(
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
            before.1
        );
    }
}

#[test]
fn older_container_only_baselines_are_partial_and_preserved_not_rewritten() {
    for relation in ["history", "project"] {
        let (_temp, vault, store, memory, expected) = fixture();
        let link = if relation == "history" {
            history_link(memory)
        } else {
            project().link()
        };
        let doc = MemoryDocument::parse(memory, &expected.accepted_markdown).unwrap();
        let body = doc
            .body()
            .replace(&link, &format!("example\r> ~~~text\r> {link}\r> ~~~"));
        let raw = format!("{}{}", &doc.raw[..doc.raw.len() - doc.body().len()], body);
        assert!(
            MemoryDocument::parse(memory, &raw)
                .unwrap_err()
                .to_string()
                .contains("relation")
        );
        seed(&vault, memory, &raw);
        let before = (
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
        );
        let state = store.read(memory).unwrap();
        assert_eq!(state.status, "partial");
        assert_eq!(state.reason.as_deref(), Some("history_document_invalid"));
        assert!(state.accepted_markdown.is_none());
        assert_eq!(
            store
                .update(
                    memory,
                    id(2),
                    Expected {
                        revision: 1,
                        accepted_markdown: raw
                    },
                    "ignored"
                )
                .unwrap_err()
                .status,
            "partial"
        );
        let observed = store.inspect_recovery(memory, None).unwrap();
        assert!(observed.history_corrupt);
        assert_eq!(
            store
                .recover(memory, None, observed, true)
                .unwrap_err()
                .status,
            "partial"
        );
        assert_eq!(
            fs::read(path(&vault, memory, "Memoria.md")).unwrap(),
            before.0
        );
        assert_eq!(
            fs::read(path(&vault, memory, "Historial.md")).unwrap(),
            before.1
        );
        assert!(!path(&vault, memory, ".pending.json").exists());
    }
}
