use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_checkpoint::CheckpointSaver;
use langgraph_checkpoint_sqlite::SqliteSaver;
use langgraph_core::{StateGraph, Topic};
use langgraph_pregel::CheckpointedSequentialExecutor;
use serde_json::json;
use tempfile::tempdir;

fn build_topic_round_graph() -> langgraph_core::CompiledGraph {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "start",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("tick"))]))),
        )
        .expect("start should be inserted");
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).expect("finish inserted");
    graph.add_edge("start", "finish").expect("edge should be valid");
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(Topic));
    graph.compile().expect("graph should compile")
}

#[test]
fn sqlite_checkpoint_bridge_invocation_and_resume_latest_work() {
    let dir = tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("checkpoint-bridge.db");
    let saver = Arc::new(SqliteSaver::new(&db_path).expect("sqlite saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver.clone());
    let graph = build_topic_round_graph();

    let first = runner
        .invoke_thread(&graph, "thread-sqlite-1", BTreeMap::new())
        .expect("first invoke should succeed");
    let resumed =
        runner.resume_latest(&graph, "thread-sqlite-1").expect("resume_latest should succeed");

    assert_eq!(first.execution.state.get("events"), Some(&json!(["tick"])));
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(first.execution.metadata.versions_seen.get("events"), Some(&1));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));

    let checkpoints = saver.list("thread-sqlite-1").expect("sqlite list should succeed");
    assert_eq!(checkpoints.len(), 2);
}

#[test]
fn sqlite_checkpoint_bridge_resume_by_id_from_specific_snapshot() {
    let dir = tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("checkpoint-bridge.db");
    let saver = Arc::new(SqliteSaver::new(&db_path).expect("sqlite saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver);
    let graph = build_topic_round_graph();

    let first = runner
        .invoke_thread(&graph, "thread-sqlite-2", BTreeMap::new())
        .expect("first invoke should succeed");
    let resumed = runner
        .resume(&graph, "thread-sqlite-2", first.checkpoint_id.clone())
        .expect("resume by id should succeed");

    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
}
