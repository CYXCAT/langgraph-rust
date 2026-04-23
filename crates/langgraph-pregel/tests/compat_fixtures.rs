use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use langgraph_core::StateGraph;
use langgraph_pregel::SequentialExecutor;
use serde_json::{json, Value};

fn fixture_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat").join(file_name)
}

fn load_json_fixture(file_name: &str) -> Value {
    let path = fixture_path(file_name);
    let content = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    });
    serde_json::from_str(&content).unwrap_or_else(|err| {
        panic!("failed to parse fixture {}: {err}", path.display());
    })
}

fn build_linear_text_graph() -> StateGraph {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "node_a",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("a"))]))),
        )
        .expect("node_a should be inserted");
    graph
        .add_node(
            "node_b",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("b"))]))),
        )
        .expect("node_b should be inserted");
    graph.add_edge("node_a", "node_b").expect("edge should be valid");
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
    graph
}

#[test]
fn compat_linear_graph_fixture_matches_expected_output() {
    let input = load_json_fixture("linear_graph_input.json");
    let expected = load_json_fixture("linear_graph_expected.json");

    let input_state = input
        .get("input_state")
        .and_then(Value::as_object)
        .expect("input_state should be a json object");
    let mut initial_state = BTreeMap::new();
    for (key, value) in input_state {
        initial_state.insert(key.clone(), value.clone());
    }

    let compiled = build_linear_text_graph().compile().expect("graph should compile");
    let final_state =
        SequentialExecutor.invoke(&compiled, initial_state).expect("execution should succeed");

    let expected_state = expected
        .get("final_state")
        .and_then(Value::as_object)
        .expect("final_state should be a json object");
    for (key, expected_value) in expected_state {
        assert_eq!(
            final_state.get(key),
            Some(expected_value),
            "state field `{key}` does not match fixture expectation"
        );
    }
}
