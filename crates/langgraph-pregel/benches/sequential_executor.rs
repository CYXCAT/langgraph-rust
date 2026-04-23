use std::collections::BTreeMap;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use langgraph_core::StateGraph;
use langgraph_pregel::SequentialExecutor;
use serde_json::json;

fn bench_invoke(c: &mut Criterion) {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "node_a",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("a"))]))),
        )
        .expect("node_a");
    graph
        .add_node(
            "node_b",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("text"), json!("b"))]))),
        )
        .expect("node_b");
    graph.add_edge("node_a", "node_b").expect("edge");
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
    let compiled = graph.compile().expect("compile");

    c.bench_function("sequential invoke", |b| {
        b.iter(|| {
            let initial = BTreeMap::from([(String::from("text"), json!(""))]);
            let _ = SequentialExecutor.invoke(&compiled, initial).expect("invoke");
        })
    });
}

criterion_group!(benches, bench_invoke);
criterion_main!(benches);
