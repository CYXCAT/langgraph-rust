use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{BinaryOperatorAggregate, GraphError, LastValue, StateGraph};
use langgraph_pregel::SequentialExecutor;
use serde_json::json;

#[test]
fn invoke_matches_readme_style_example() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "node_a",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("a"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "node_b",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("b"))]))),
        )
        .unwrap();

    graph.add_edge("node_a", "node_b").unwrap();
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

    let initial = BTreeMap::from([(String::from("text"), json!(""))]);
    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, initial).unwrap();

    assert_eq!(final_state.get("text"), Some(&json!("ab")));
}

#[test]
fn compile_rejects_duplicate_nodes() {
    let mut graph = StateGraph::new();
    graph.add_node("node_a", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    let err = graph.add_node("node_a", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap_err();
    assert!(matches!(err, GraphError::DuplicateNode(_)));
}

#[test]
fn invoke_supports_branch_merge_with_channel_aggregate() {
    let mut graph = StateGraph::new();
    graph.add_node("start", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph
        .add_node(
            "branch_a",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("sum"), json!(1))]))),
        )
        .unwrap();
    graph
        .add_node(
            "branch_b",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("sum"), json!(2))]))),
        )
        .unwrap();
    graph
        .add_node(
            "finish",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("done"), json!(true))]))),
        )
        .unwrap();

    graph.add_edge("start", "branch_a").unwrap();
    graph.add_edge("start", "branch_b").unwrap();
    graph.add_edge("branch_a", "finish").unwrap();
    graph.add_edge("branch_b", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel(
        "sum",
        Arc::new(BinaryOperatorAggregate::new(Arc::new(|left, right| {
            json!(left.as_i64().unwrap_or_default() + right.as_i64().unwrap_or_default())
        }))),
    );

    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap();
    assert_eq!(final_state.get("sum"), Some(&json!(3)));
    assert_eq!(final_state.get("done"), Some(&json!(true)));
}

#[test]
fn invoke_uses_last_value_channel_in_same_superstep() {
    let mut graph = StateGraph::new();
    graph.add_node("start", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph
        .add_node(
            "branch_a",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("status"), json!("a"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "branch_b",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("status"), json!("b"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();

    graph.add_edge("start", "branch_a").unwrap();
    graph.add_edge("start", "branch_b").unwrap();
    graph.add_edge("branch_a", "finish").unwrap();
    graph.add_edge("branch_b", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("status", Arc::new(LastValue));

    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap();
    assert_eq!(final_state.get("status"), Some(&json!("b")));
}
