use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{CompiledGraph, GraphError, StateGraph};
use serde_json::json;

use crate::error::CliError;

pub fn build_graph(name: &str) -> Result<CompiledGraph, CliError> {
    match name {
        "append_ab" => build_append_ab_graph().map_err(CliError::GraphCompile),
        _ => Err(CliError::UnknownGraphPreset(name.to_string())),
    }
}

fn build_append_ab_graph() -> Result<CompiledGraph, GraphError> {
    let mut graph = StateGraph::new();
    graph.add_node(
        "node_a",
        Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("a"))]))),
    )?;
    graph.add_node(
        "node_b",
        Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("b"))]))),
    )?;
    graph.add_edge("node_a", "node_b")?;
    graph.set_entry_point("node_a");
    graph.set_finish_point("node_b");
    graph.add_reducer(
        "text",
        Arc::new(|left, right| {
            let lhs = left.as_str().unwrap_or_default();
            let rhs = right.as_str().unwrap_or_default();
            json!(format!("{lhs}{rhs}"))
        }),
    );
    graph.compile()
}
