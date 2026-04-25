use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use langgraph_checkpoint_postgres::PostgresSaver;
use langgraph_core::{NodeOutput, RuntimeContext, StateGraph, Topic};
use langgraph_pregel::{
    CheckpointBridgeError, CheckpointedSequentialExecutor, StreamEvent, ThreadInvocationOutcome,
    ThreadStreamEvent,
};
use serde_json::json;

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

fn build_interrupt_graph() -> langgraph_core::CompiledGraph {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "gate",
            Arc::new(|_state, runtime_context| {
                let marker = runtime_context
                    .get("marker")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("default");
                Ok(NodeOutput::interrupt(BTreeMap::from([(String::from("events"), json!(marker))])))
            }),
        )
        .expect("gate should be inserted");
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).expect("finish inserted");
    graph.add_edge("gate", "finish").expect("edge should be valid");
    graph.set_entry_point("gate");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(Topic));
    graph.compile().expect("graph should compile")
}

fn test_database_url() -> Option<String> {
    env::var("LANGGRAPH_POSTGRES_TEST_URL").ok()
}

fn unique_thread_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    format!("{prefix}-{nanos}")
}

async fn collect_thread_stream(
    mut rx: tokio::sync::mpsc::Receiver<Result<ThreadStreamEvent, CheckpointBridgeError>>,
) -> (Vec<StreamEvent>, String) {
    let mut events = Vec::<StreamEvent>::new();
    let mut checkpoint_id: Option<String> = None;
    while let Some(item) = rx.recv().await {
        match item.expect("thread stream should not error") {
            ThreadStreamEvent::Execution(event) => events.push(event),
            ThreadStreamEvent::CheckpointPersisted { checkpoint_id: persisted } => {
                checkpoint_id = Some(persisted);
            }
        }
    }
    let checkpoint_id = checkpoint_id.expect("thread stream should emit checkpoint id");
    (events, checkpoint_id)
}

#[test]
fn postgres_checkpoint_bridge_invocation_and_resume_latest_work() {
    let Some(url) = test_database_url() else {
        return;
    };
    let saver = Arc::new(PostgresSaver::new(url).expect("postgres saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver);
    let graph = build_topic_round_graph();
    let thread_id = unique_thread_id("thread-pregel-pg-invoke");

    let first = runner
        .invoke_thread(&graph, thread_id.clone(), BTreeMap::new())
        .expect("first invoke should succeed");
    let resumed = runner.resume_latest(&graph, thread_id).expect("resume_latest should succeed");

    assert_eq!(first.execution.state.get("events"), Some(&json!(["tick"])));
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(first.execution.metadata.versions_seen.get("events"), Some(&1));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
}

#[test]
fn postgres_checkpoint_bridge_interruption_persists_and_supports_resume() {
    let Some(url) = test_database_url() else {
        return;
    };
    let saver = Arc::new(PostgresSaver::new(url).expect("postgres saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver);
    let interrupt_graph = build_interrupt_graph();
    let thread_id = unique_thread_id("thread-pregel-pg-interrupt");

    let outcome = runner
        .invoke_thread_with_runtime_context(
            &interrupt_graph,
            thread_id.clone(),
            BTreeMap::new(),
            RuntimeContext::from([(String::from("marker"), json!("halt"))]),
        )
        .expect("interrupt invoke should return outcome");
    let interrupted = match outcome {
        ThreadInvocationOutcome::Interrupted(interrupted) => interrupted,
        ThreadInvocationOutcome::Completed(_) => panic!("expected interrupted outcome"),
    };
    assert_eq!(interrupted.interrupt.node, "gate");
    assert_eq!(interrupted.interrupt.superstep, 1);

    let resumed = runner
        .resume_latest(&build_topic_round_graph(), thread_id.clone())
        .expect("resume_latest should continue from interrupted checkpoint");
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["halt", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));

    let old_api_err = runner
        .invoke_thread(&interrupt_graph, thread_id, BTreeMap::new())
        .expect_err("old invoke_thread API should return interrupted error");
    assert!(matches!(old_api_err, CheckpointBridgeError::Interrupted { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn postgres_async_stream_thread_persists_checkpoint_and_supports_resume_latest() {
    let Some(url) = test_database_url() else {
        return;
    };
    let saver = Arc::new(PostgresSaver::new(url).expect("postgres saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver);
    let graph = build_topic_round_graph();
    let thread_id = unique_thread_id("thread-pregel-pg-astream");

    let stream = runner
        .astream_thread(&graph, thread_id.clone(), BTreeMap::new())
        .await
        .expect("astream_thread should succeed");
    let (events, checkpoint_id) = collect_thread_stream(stream).await;
    let completed = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::Completed { state, interrupt, .. } => {
                Some((state.clone(), interrupt.clone()))
            }
            _ => None,
        })
        .expect("stream should emit completed event");
    assert_eq!(completed.0.get("events"), Some(&json!(["tick"])));
    assert!(completed.1.is_none());

    let resumed = runner
        .aresume_latest(&graph, thread_id)
        .await
        .expect("aresume_latest should consume streamed checkpoint");
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
    assert!(!checkpoint_id.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn postgres_async_stream_interrupt_persists_checkpoint_and_supports_resume_latest() {
    let Some(url) = test_database_url() else {
        return;
    };
    let saver = Arc::new(PostgresSaver::new(url).expect("postgres saver should init"));
    let runner = CheckpointedSequentialExecutor::new(saver);
    let interrupt_graph = build_interrupt_graph();
    let thread_id = unique_thread_id("thread-pregel-pg-astream-interrupt");

    let stream = runner
        .astream_thread_with_runtime_context(
            &interrupt_graph,
            thread_id.clone(),
            BTreeMap::new(),
            RuntimeContext::from([(String::from("marker"), json!("stream-halt"))]),
        )
        .await
        .expect("astream interrupt should persist checkpoint");
    let (events, checkpoint_id) = collect_thread_stream(stream).await;
    let completed_interrupt = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::Completed { interrupt, .. } => {
                Some(interrupt.as_ref().map(|signal| signal.node.clone()))
            }
            _ => None,
        })
        .expect("stream should emit completed event");
    assert_eq!(completed_interrupt.as_deref(), Some("gate"));

    let resumed = runner
        .aresume_latest(&build_topic_round_graph(), thread_id)
        .await
        .expect("aresume_latest should resume from interrupted streamed checkpoint");
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["stream-halt", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
    assert!(!checkpoint_id.is_empty());
}
