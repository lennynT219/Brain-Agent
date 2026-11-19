use crate::{memory::store::Failure, projects::{self, Baseline, Preview, Project}};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use uuid::Uuid;

const MAX_WIRE: u64 = 4 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request { protocol_version: u32, request_id: Uuid, operation: String, arguments: Value }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity { project_id: Uuid }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft { title: String, body: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bind { project_id: Uuid, expected_project: Project, expected: Baseline, human_approved: bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create { preview: Preview, expected: Baseline, human_approved: bool }
fn failure(status: &str, message: &str) -> Failure {
    Failure { status: status.into(), message: message.into(), memory_id: None, io_kind: None, reason: None }
}
fn args<T: DeserializeOwned>(value: Value) -> Result<T, Failure> {
    serde_json::from_value(value).map_err(|_| failure("invalid", "invalid Projects arguments"))
}
fn dispatch(request: Request) -> Result<Value, Failure> {
    if request.protocol_version != 1 { return Err(failure("unsupported", "unsupported protocol version")); }
    if request.request_id.is_nil() { return Err(failure("invalid", "request UUID must be nonnil")); }
    let a = request.arguments;
    match request.operation.as_str() {
        "capabilities" => { args::<Empty>(a)?; Ok(json!({"status":"found","protocol_version":1,
            "operations":["capabilities","status","list","read","preview_create","draft_context","bind","create_and_bind"],
            "binding_version":2,"max_input_bytes":MAX_WIRE,"max_output_bytes":MAX_WIRE,
            "history_response_bytes":32768,"history_formats":[1,2],"authority":"trusted_local_host",
            "atomicity":"cooperative_locks; no_multi_file_transaction; uncertain_mutation_must_not_retry"})) },
        "status" => { args::<Empty>(a)?; projects::status() },
        "list" => { args::<Empty>(a)?; projects::list() },
        "read" => projects::read(args::<Identity>(a)?.project_id),
        "draft_context" => { args::<Empty>(a)?; projects::draft_context() },
        "preview_create" => { let Draft { title, body } = args(a)?; projects::preview_create(title, body) },
        "bind" => { let Bind { project_id, expected_project, expected, human_approved } = args(a)?;
            projects::bind(request.request_id, project_id, None, Some(expected_project), expected, human_approved) },
        "create_and_bind" => { let Create { preview, expected, human_approved } = args(a)?;
            let id = preview.project_id();
            projects::bind(request.request_id, id, Some(preview), None, expected, human_approved) },
        _ => Err(failure("unsupported", "unsupported Projects operation")),
    }
}
pub(crate) fn execute() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    io::stdin().lock().take(MAX_WIRE + 1).read_to_end(&mut bytes)?;
    let parsed = if bytes.len() as u64 > MAX_WIRE { Err("too_large") }
        else { serde_json::from_slice::<Request>(&bytes).map_err(|_| "invalid") };
    let (id, op, result) = match parsed {
        Ok(request) if request.operation.len() <= 32 => {
            let id = request.request_id;
            let operation = request.operation.clone();
            let result = match dispatch(request) {
                Ok(v) => v,
                Err(e) => serde_json::to_value(e)?,
            };
            (Some(id), Some(operation), result)
        }
        Ok(_) => (None, None, json!({"status":"invalid","message":"operation exceeds bound"})),
        Err(status) => (None, None, json!({"status":status,"message":"invalid Projects envelope or transport limit"})),
    };
    let envelope = |result| json!({"protocol_version":1,"request_id":id,"operation":op,"result":result});
    let mut output = serde_json::to_vec(&envelope(result))?;
    if output.len() as u64 + 1 > MAX_WIRE {
        output = serde_json::to_vec(&envelope(json!({"status":"too_large","message":"complete output exceeds bound; not truncated"})))?;
    }
    output.push(b'\n');
    let mut stdout = io::stdout().lock();
    stdout.write_all(&output)?;
    stdout.flush()?;
    Ok(())
}
