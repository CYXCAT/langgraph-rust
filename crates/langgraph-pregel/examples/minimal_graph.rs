use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::StateGraph;
use langgraph_pregel::SequentialExecutor;
use serde_json::json;

fn main() {
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

    let compiled = graph.compile().expect("graph should compile");
    let final_state = SequentialExecutor
        .invoke(&compiled, BTreeMap::from([(String::from("text"), json!(""))]))
        .expect("execution should succeed");

    println!("{}", json!(final_state));
}
