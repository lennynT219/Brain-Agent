use crate::notes::{self, Failure, Scope};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use uuid::Uuid;

const MAX_WIRE: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: u32,
    request_id: Uuid,
    operation: String,
    arguments: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListNotes {
    #[serde(default)]
    project_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadNote {
    note_id: Uuid,
    #[serde(default)]
    project_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNote {
    project_id: Uuid,
    title: String,
    body: String,
    // Hosts send true to fail closed with older binaries that reject the
    // unknown field. Omission stays compatible for existing clients.
    #[serde(default)]
    require_named_link: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateNote {
    project_id: Uuid,
    note_id: Uuid,
    expected_markdown: String,
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameNote {
    project_id: Uuid,
    note_id: Uuid,
    expected_markdown: String,
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectRename {
    project_id: Uuid,
}

fn arguments<T: DeserializeOwned>(value: Value) -> Result<T, Failure> {
    serde_json::from_value(value).map_err(|_| Failure("invalid", "invalid operation arguments"))
}

fn dispatch(request: Request) -> Result<Value, Failure> {
    if request.protocol_version != 1 {
        return Err(Failure("unsupported", "unsupported protocol version"));
    }
    if request.request_id.is_nil() {
        return Err(Failure("invalid", "request ID must be a nonnil UUID"));
    }
    match request.operation.as_str() {
        "list" => {
            let ListNotes { project_id } = arguments(request.arguments)?;
            match project_id {
                Some(id) => { notes::validate_id(id)?; Scope::load_additional(id)?.list() }
                None => Scope::load()?.list(),
            }
        }
        "read" => {
            let ReadNote { note_id, project_id } = arguments(request.arguments)?;
            notes::validate_id(note_id)?;
            match project_id {
                Some(id) => { notes::validate_id(id)?; Scope::load_additional(id)?.read(note_id) }
                None => Scope::load()?.read(note_id),
            }
        }
        "create" => {
            let CreateNote {
                project_id,
                title,
                body,
                require_named_link,
            } = arguments(request.arguments)?;
            if require_named_link == Some(false) {
                return Err(Failure("invalid", "Notes are always created with a title filename and Project link"));
            }
            notes::validate_id(project_id)?;
            Scope::load()?.create(project_id, title, &body)
        }
        "update" => {
            let UpdateNote {
                project_id,
                note_id,
                expected_markdown,
                body,
            } = arguments(request.arguments)?;
            notes::validate_id(project_id)?;
            notes::validate_id(note_id)?;
            Scope::load()?.update(project_id, note_id, &expected_markdown, &body)
        }
        "rename" => {
            let RenameNote { project_id, note_id, expected_markdown, title } = arguments(request.arguments)?;
            notes::validate_id(project_id)?;
            notes::validate_id(note_id)?;
            Scope::load()?.rename(project_id, note_id, &expected_markdown, &title)
        }
        "inspect_rename" => {
            let InspectRename { project_id } = arguments(request.arguments)?;
            notes::validate_id(project_id)?;
            Scope::load()?.inspect_rename(project_id)
        }
        _ => Err(Failure("unsupported", "unsupported Notes operation")),
    }
}

fn response(id: Option<Uuid>, result: Result<Value, Failure>) -> Value {
    let result = result
        .unwrap_or_else(|Failure(status, message)| json!({"status": status, "message": message}));
    json!({"protocol_version": 1, "request_id": id, "result": result})
}

pub(crate) fn execute() -> anyhow::Result<()> {
    let mut input = Vec::new();
    io::stdin()
        .lock()
        .take(MAX_WIRE + 1)
        .read_to_end(&mut input)?;
    let (id, result) = if input.len() as u64 > MAX_WIRE {
        (
            None,
            Err(Failure("input_limit", "request exceeds byte limit")),
        )
    } else {
        match serde_json::from_slice::<Request>(&input) {
            Ok(request) => (Some(request.request_id), dispatch(request)),
            Err(_) => (
                None,
                Err(Failure("invalid", "invalid Notes request envelope")),
            ),
        }
    };
    let mut bytes = serde_json::to_vec(&response(id, result))?;
    // A publication receipt consists only of a status, two UUIDs and a bounded title.
    // Only reads can exceed the output limit.
    if bytes.len() as u64 >= MAX_WIRE {
        bytes = serde_json::to_vec(&response(
            id,
            Err(Failure("output_limit", "response exceeds byte limit")),
        ))?;
    }
    let mut stdout = io::stdout().lock();
    bytes.push(b'\n');
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}
