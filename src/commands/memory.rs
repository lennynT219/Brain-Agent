use crate::memory::{
    markdown::SectionEdit,
    store::{Context, Create, Expected, Failure, Observation, ReplaceMissing, Store},
};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: u32,
    request_id: Uuid,
    operation: String,
    #[serde(default)]
    vault: Option<PathBuf>,
    arguments: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    memory_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    memory_id: Uuid,
    expected: Expected,
    body: String,
}
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum SectionUpdate {
    Changes {
        memory_id: Uuid,
        expected: Expected,
        changes: BTreeMap<String, String>,
        summary: String,
        // A protocol precondition for callers that must reject older Brain
        // binaries; this implementation preserves separators either way.
        #[serde(
            default,
            rename = "require_preserved_separator",
            deserialize_with = "require_true"
        )]
        _require_preserved_separator: bool,
    },
    Edits {
        memory_id: Uuid,
        expected: Expected,
        edits: Vec<SectionEdit>,
        summary: String,
    },
}
fn require_true<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    if bool::deserialize(deserializer)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom("precondition must be true"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    memory_id: Uuid,
    expected: Expected,
    observed_markdown: String,
    approved_markdown: String,
    human_approved: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspection {
    memory_id: Uuid,
    #[serde(default)]
    context: Option<Context>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    memory_id: Uuid,
    #[serde(default)]
    context: Option<Context>,
    observation: Observation,
    human_approved: bool,
}

fn failure(status: &str, message: &str) -> Failure {
    Failure {
        status: status.into(),
        message: message.into(),
        memory_id: None,
        io_kind: None,
        reason: None,
    }
}
fn args<T: serde::de::DeserializeOwned>(value: Value) -> std::result::Result<T, Failure> {
    serde_json::from_value(value).map_err(|_| failure("invalid", "invalid operation arguments"))
}
fn value<T: serde::Serialize>(result: std::result::Result<T, Failure>) -> Value {
    match result {
        Ok(value) => serde_json::to_value(value).expect("serializable typed result"),
        Err(error) => serde_json::to_value(error).expect("serializable failure"),
    }
}

fn dispatch(request: Request, max_input: u64, max_output: u64) -> Value {
    if request.protocol_version != 1 {
        return json!({"status":"unsupported","message":"unsupported protocol version"});
    }
    if request.request_id.is_nil() {
        return json!({"status":"invalid","message":"request ID must be nonnil UUID"});
    }
    if request.operation == "capabilities" {
        if args::<Empty>(request.arguments).is_err() {
            return json!({"status":"invalid","message":"capabilities takes empty arguments"});
        }
        return json!({"status":"found","protocol_version":1,"brain_version":env!("CARGO_PKG_VERSION"),"operations":["capabilities","resolve","read","create","replace_missing","update","accept_external","inspect_recovery","recover","update_sections"],"max_input_bytes":max_input,"max_output_bytes":max_output,"history_version":2,"document_version":1,"lock":"std_try_lock","authority":"trusted_local_caller","cancellation":"host_terminates_process; mutation_confirmation_may_be_lost"});
    }
    if !matches!(
        request.operation.as_str(),
        "resolve"
            | "read"
            | "create"
            | "replace_missing"
            | "update"
            | "accept_external"
            | "inspect_recovery"
            | "recover"
            | "update_sections"
    ) {
        return json!({"status":"unsupported","message":"unknown operation"});
    }
    let vault = match request.vault {
        Some(path) => path,
        None => match crate::config::load_config() {
            Ok(config) => config.vault_path,
            Err(_) => {
                return json!({"status":"unavailable","message":"selected vault configuration unavailable; no implicit initialization"});
            }
        },
    };
    let store = match Store::open(&vault) {
        Ok(store) => store,
        Err(error) => return value::<Value>(Err(error)),
    };
    let id = request.request_id;
    let arguments = request.arguments;
    match request.operation.as_str() {
        "resolve" => value(args::<Context>(arguments).and_then(|args| store.resolve(&args))),
        "read" => value(args::<Identity>(arguments).and_then(|args| store.read(args.memory_id))),
        "create" => {
            let mut arguments = arguments;
            let summary = arguments
                .as_object_mut()
                .and_then(|args| args.remove("summary"));
            value(args::<Create>(arguments).and_then(|args| match summary {
                Some(Value::String(summary)) => store.create_log(id, args, &summary),
                None => store.create(id, args),
                Some(_) => Err(failure("invalid", "invalid history summary")),
            }))
        }
        "replace_missing" => value(
            args::<ReplaceMissing>(arguments).and_then(|args| store.replace_missing(id, args)),
        ),
        "update" => value(
            args::<Update>(arguments)
                .and_then(|args| store.update(args.memory_id, id, args.expected, &args.body)),
        ),
        "update_sections" => value(args::<SectionUpdate>(arguments).and_then(|args| match args {
            SectionUpdate::Changes {
                memory_id,
                expected,
                changes,
                summary,
                ..
            } => store.update_sections(memory_id, id, expected, &changes, &summary),
            SectionUpdate::Edits {
                memory_id,
                expected,
                edits,
                summary,
            } => store.update_section_edits(memory_id, id, expected, &edits, &summary),
        })),
        "accept_external" => value(args::<Approval>(arguments).and_then(|args| {
            store.accept_external(
                args.memory_id,
                id,
                args.expected,
                &args.observed_markdown,
                &args.approved_markdown,
                args.human_approved,
            )
        })),
        "inspect_recovery" => match args::<Inspection>(arguments)
            .and_then(|args| store.inspect_recovery(args.memory_id, args.context.as_ref()))
        {
            Ok(observation) if observation.history_corrupt => {
                json!({"status":"partial","reason":"history_corrupt","observation":observation})
            }
            Ok(observation) => json!({"status":"found","observation":observation}),
            Err(error) => value::<Value>(Err(error)),
        },
        "recover" => value(args::<Recovery>(arguments).and_then(|args| {
            store.recover(
                args.memory_id,
                args.context.as_ref(),
                args.observation,
                args.human_approved,
            )
        })),
        _ => unreachable!(),
    }
}

pub fn execute(max_input: u64, max_output: u64) -> Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(max_input + 1)
        .read_to_end(&mut bytes)?;
    let parsed = if bytes.len() as u64 > max_input {
        Err("too_large")
    } else {
        serde_json::from_slice::<Request>(&bytes).map_err(|_| "invalid")
    };
    let (id, operation, result) = match parsed {
        Ok(request) if request.operation.len() > 32 => (
            Some(request.request_id),
            None,
            json!({"status":"invalid","message":"operation name exceeds protocol bound"}),
        ),
        Ok(request) => {
            let id = Some(request.request_id);
            let operation = Some(request.operation.clone());
            (id, operation, dispatch(request, max_input, max_output))
        }
        Err(status) => (
            None,
            None,
            json!({"status":status,"message":"request exceeds transport bound or is not valid protocol JSON"}),
        ),
    };
    let envelope = |result| json!({"protocol_version":1,"request_id":id,"operation":operation,"result":result});
    let mut output = serde_json::to_vec(&envelope(result))?;
    if output.len() as u64 + 1 > max_output {
        // Mutating operations only return small typed receipts/errors; no
        // post-publication document dump can exhaust the response allowance.
        output = serde_json::to_vec(&envelope(
            json!({"status":"too_large","message":"complete response exceeds transport bound; no context was truncated","required_bytes":output.len() + 1}),
        ))?;
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(&output)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
