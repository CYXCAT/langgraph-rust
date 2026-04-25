use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use langgraph_core::{Command, State};
use langgraph_pregel::{ExecutionMetadata, InterruptSignal, StreamEvent};
use serde_json::{json, Map, Value};

use crate::error::CliError;

pub fn read_input_state(path: Option<&Path>) -> Result<State, CliError> {
    let content = read_input(path)?;
    parse_state(&content)
}

pub fn format_output(state: &State, pretty: bool) -> Result<String, CliError> {
    let value = state_to_json_value(state);
    serialize_output(&value, pretty)
}

pub fn format_output_with_metadata(
    state: &State,
    metadata: &ExecutionMetadata,
    pretty: bool,
) -> Result<String, CliError> {
    let value = json!({
        "state": state_to_json_value(state),
        "metadata": metadata_to_json_value(metadata),
    });
    serialize_output(&value, pretty)
}

pub fn format_stream_events(events: &[StreamEvent]) -> Result<String, CliError> {
    let mut lines = Vec::with_capacity(events.len());
    for event in events {
        lines.push(format_stream_event_line(event)?);
    }
    Ok(lines.join("\n"))
}

pub fn format_stream_event_line(event: &StreamEvent) -> Result<String, CliError> {
    let value = stream_event_to_json_value(event);
    serde_json::to_string(&value).map_err(CliError::SerializeOutputJson)
}

fn serialize_output(value: &Value, pretty: bool) -> Result<String, CliError> {
    if pretty {
        serde_json::to_string_pretty(value).map_err(CliError::SerializeOutputJson)
    } else {
        serde_json::to_string(value).map_err(CliError::SerializeOutputJson)
    }
}

fn read_input(path: Option<&Path>) -> Result<String, CliError> {
    match path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|source| CliError::ReadInputFile { path: path.to_path_buf(), source }),
        None => {
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content).map_err(CliError::ReadStdin)?;
            Ok(content)
        }
    }
}

fn parse_state(content: &str) -> Result<State, CliError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(BTreeMap::new());
    }
    let value: Value = serde_json::from_str(trimmed).map_err(CliError::ParseInputJson)?;
    let Value::Object(map) = value else {
        return Err(CliError::InputMustBeJsonObject);
    };
    Ok(map.into_iter().collect())
}

fn state_to_json_value(state: &State) -> Value {
    let object: Map<String, Value> =
        state.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
    Value::Object(object)
}

fn metadata_to_json_value(metadata: &ExecutionMetadata) -> Value {
    let command_trace: Vec<Value> = metadata
        .command_trace
        .iter()
        .map(|event| {
            json!({
                "node": event.node,
                "superstep": event.superstep,
                "command": command_to_json_value(&event.command),
            })
        })
        .collect();
    json!({
        "supersteps": metadata.supersteps,
        "channel_versions": metadata.channel_versions,
        "versions_seen": metadata.versions_seen,
        "command_trace": command_trace,
    })
}

fn command_to_json_value(command: &Command) -> Value {
    match command {
        Command::Interrupt => json!({ "type": "interrupt" }),
        Command::Goto(target) => json!({ "type": "goto", "target": target }),
        Command::GotoMany(targets) => json!({ "type": "goto_many", "targets": targets }),
    }
}

fn stream_event_to_json_value(event: &StreamEvent) -> Value {
    match event {
        StreamEvent::NodeStarted { node, superstep } => {
            json!({ "event": "node_started", "node": node, "superstep": superstep })
        }
        StreamEvent::NodeFinished { node, superstep, patch } => {
            json!({
                "event": "node_finished",
                "node": node,
                "superstep": superstep,
                "patch": patch,
            })
        }
        StreamEvent::StateChunk { superstep, chunk } => {
            json!({ "event": "state_chunk", "superstep": superstep, "chunk": chunk })
        }
        StreamEvent::CommandEmitted { node, superstep, command } => {
            json!({
                "event": "command_emitted",
                "node": node,
                "superstep": superstep,
                "command": command_to_json_value(command),
            })
        }
        StreamEvent::Interrupted { node, superstep } => {
            json!({ "event": "interrupted", "node": node, "superstep": superstep })
        }
        StreamEvent::Completed { state, metadata, interrupt } => {
            json!({
                "event": "completed",
                "state": state_to_json_value(state),
                "metadata": metadata_to_json_value(metadata),
                "interrupt": interrupt_to_json_value(interrupt),
            })
        }
    }
}

fn interrupt_to_json_value(interrupt: &Option<InterruptSignal>) -> Value {
    match interrupt {
        Some(interrupt) => {
            json!({ "node": interrupt.node, "superstep": interrupt.superstep })
        }
        None => Value::Null,
    }
}
