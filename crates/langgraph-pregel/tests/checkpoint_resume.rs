use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver, InMemorySaver, PendingWrite};
use langgraph_core::{Command, NodeOutput, RuntimeContext, StateGraph, Topic};
use langgraph_pregel::{
    CheckpointBridgeError, CheckpointedSequentialExecutor, StreamEvent, ThreadInvocationOutcome,
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
                Ok(NodeOutput::interrupt(BTreeMap::from([(
                    String::from("events"),
                    json!(marker),
                )])))
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

#[derive(Default)]
struct ReverseListSaver {
    inner: InMemorySaver,
}

impl ReverseListSaver {
    fn new() -> Self {
        Self::default()
    }
}

impl CheckpointSaver for ReverseListSaver {
    fn put(&self, checkpoint: Checkpoint) -> Result<(), CheckpointError> {
        self.inner.put(checkpoint)
    }

    fn get(&self, thread_id: &str, checkpoint_id: &str) -> Result<Option<Checkpoint>, CheckpointError> {
        self.inner.get(thread_id, checkpoint_id)
    }

    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError> {
        let mut items = self.inner.list(thread_id)?;
        items.reverse();
        Ok(items)
    }
}

#[test]
fn checkpoint_records_versions_seen_from_execution_metadata() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let run = runner
        .invoke_thread(&graph, "thread-1", BTreeMap::new())
        .expect("first run should succeed");

    assert_eq!(run.execution.metadata.versions_seen.get("events"), Some(&1));
    let checkpoint = saver
        .get("thread-1", &run.checkpoint_id)
        .expect("checkpoint lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(
        checkpoint.versions_seen.get("events"),
        run.execution.metadata.versions_seen.get("events")
    );
}

#[test]
fn resume_latest_continues_thread_and_channel_versions() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let first = runner
        .invoke_thread(&graph, "thread-1", BTreeMap::new())
        .expect("first run should succeed");
    let second = runner
        .resume_latest(&graph, "thread-1")
        .expect("resume should succeed");

    assert_eq!(first.execution.state.get("events"), Some(&json!(["tick"])));
    assert_eq!(second.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(first.execution.metadata.versions_seen.get("events"), Some(&1));
    assert_eq!(second.execution.metadata.versions_seen.get("events"), Some(&2));

    let checkpoints = saver.list("thread-1").expect("list should work");
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].checkpoint_id, first.checkpoint_id);
    assert_eq!(checkpoints[1].checkpoint_id, second.checkpoint_id);
}

#[test]
fn resume_by_checkpoint_id_replays_from_specific_snapshot() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let first = runner
        .invoke_thread(&graph, "thread-1", BTreeMap::new())
        .expect("first run should succeed");
    let resumed = runner
        .resume(&graph, "thread-1", first.checkpoint_id.clone())
        .expect("resume by id should succeed");

    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
}

#[test]
fn resume_materializes_pending_writes_before_continuing_execution() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let interrupted = Checkpoint {
        thread_id: String::from("thread-2"),
        checkpoint_id: String::from("cp-0001"),
        state: BTreeMap::from([(String::from("events"), json!(["seed"]))]),
        versions_seen: BTreeMap::from([(String::from("events"), 1)]),
        pending_writes: vec![PendingWrite {
            node: String::from("start"),
            patch: BTreeMap::from([(String::from("events"), json!("pending"))]),
        }],
    };
    saver
        .put(interrupted)
        .expect("seed interrupted checkpoint should succeed");

    let resumed = runner
        .resume_latest(&graph, "thread-2")
        .expect("resume_latest should materialize pending writes");

    assert_eq!(
        resumed.execution.state.get("events"),
        Some(&json!(["seed", "pending", "tick"]))
    );
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&3));

    let checkpoints = saver.list("thread-2").expect("list should work");
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints[1].pending_writes.is_empty());
}

#[test]
fn interrupt_then_resume_latest_forms_minimal_durable_loop() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let interrupted = runner
        .interrupt_thread(
            "thread-3",
            BTreeMap::from([(String::from("events"), json!(["seed"]))]),
            BTreeMap::from([(String::from("events"), 1)]),
            vec![PendingWrite {
                node: String::from("start"),
                patch: BTreeMap::from([(String::from("events"), json!("interrupted"))]),
            }],
        )
        .expect("interrupt checkpoint should be persisted");
    assert_eq!(interrupted.checkpoint_id, "cp-0001");

    let saved = saver
        .get("thread-3", &interrupted.checkpoint_id)
        .expect("lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(saved.pending_writes.len(), 1);

    let resumed = runner
        .resume_latest(&graph, "thread-3")
        .expect("resume from interrupted checkpoint should succeed");

    assert_eq!(
        resumed.execution.state.get("events"),
        Some(&json!(["seed", "interrupted", "tick"]))
    );
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&3));

    let checkpoints = saver.list("thread-3").expect("list should work");
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints[1].pending_writes.is_empty());
}

#[test]
fn resume_latest_uses_numeric_checkpoint_order_instead_of_lexicographic_order() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    saver
        .put(Checkpoint {
            thread_id: String::from("thread-4"),
            checkpoint_id: String::from("cp-9999"),
            state: BTreeMap::from([(String::from("events"), json!(["old"]))]),
            versions_seen: BTreeMap::from([(String::from("events"), 9999)]),
            pending_writes: vec![],
        })
        .expect("older checkpoint should be persisted");
    saver
        .put(Checkpoint {
            thread_id: String::from("thread-4"),
            checkpoint_id: String::from("cp-10000"),
            state: BTreeMap::from([(String::from("events"), json!(["new"]))]),
            versions_seen: BTreeMap::from([(String::from("events"), 10000)]),
            pending_writes: vec![],
        })
        .expect("newer checkpoint should be persisted");

    let resumed = runner
        .resume_latest(&graph, "thread-4")
        .expect("resume_latest should choose cp-10000 as latest");

    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["new", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&10001));
}

#[test]
fn resume_latest_is_stable_even_if_backend_list_order_is_unsorted() {
    let saver = ReverseListSaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    saver
        .put(Checkpoint {
            thread_id: String::from("thread-5"),
            checkpoint_id: String::from("cp-0001"),
            state: BTreeMap::from([(String::from("events"), json!(["v1"]))]),
            versions_seen: BTreeMap::from([(String::from("events"), 1)]),
            pending_writes: vec![],
        })
        .expect("cp-0001 should be persisted");
    saver
        .put(Checkpoint {
            thread_id: String::from("thread-5"),
            checkpoint_id: String::from("cp-0002"),
            state: BTreeMap::from([(String::from("events"), json!(["v2"]))]),
            versions_seen: BTreeMap::from([(String::from("events"), 2)]),
            pending_writes: vec![],
        })
        .expect("cp-0002 should be persisted");

    let resumed = runner
        .resume_latest(&graph, "thread-5")
        .expect("resume_latest should choose numeric latest checkpoint");

    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["v2", "tick"])));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&3));
}

#[test]
fn invoke_thread_with_runtime_context_persists_checkpoint_on_interrupt() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_interrupt_graph();

    let runtime_context = RuntimeContext::from([(String::from("marker"), json!("halt"))]);
    let result = runner
        .invoke_thread_with_runtime_context(&graph, "thread-6", BTreeMap::new(), runtime_context)
        .expect("invoke should persist interrupted checkpoint");

    let interrupted = match result {
        ThreadInvocationOutcome::Interrupted(interrupted) => interrupted,
        ThreadInvocationOutcome::Completed(_) => panic!("expected interrupted outcome"),
    };
    assert_eq!(interrupted.interrupt.node, "gate");
    assert_eq!(interrupted.interrupt.superstep, 1);
    assert_eq!(interrupted.command_trace.len(), 1);
    assert!(matches!(
        interrupted.command_trace[0].command,
        Command::Interrupt
    ));

    let checkpoint = saver
        .get("thread-6", &interrupted.checkpoint_id)
        .expect("lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(checkpoint.state.get("events"), Some(&json!(["halt"])));
    assert_eq!(checkpoint.versions_seen.get("events"), Some(&1));
    assert!(checkpoint.pending_writes.is_empty());
}

#[test]
fn invoke_thread_returns_interrupted_error_and_leaves_resumable_checkpoint() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_interrupt_graph();

    let error = runner.invoke_thread(&graph, "thread-7", BTreeMap::new()).unwrap_err();
    let checkpoint_id = match error {
        CheckpointBridgeError::Interrupted {
            thread_id,
            checkpoint_id,
            node,
            superstep,
        } => {
            assert_eq!(thread_id, "thread-7");
            assert_eq!(node, "gate");
            assert_eq!(superstep, 1);
            checkpoint_id
        }
        other => panic!("unexpected error: {other}"),
    };

    let checkpoint = saver
        .get("thread-7", &checkpoint_id)
        .expect("lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(checkpoint.state.get("events"), Some(&json!(["default"])));
    assert_eq!(checkpoint.versions_seen.get("events"), Some(&1));
}

#[test]
fn command_audit_hook_can_reject_execution_before_checkpoint_persist() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver).with_command_audit_hook(Arc::new(|event| {
        if matches!(event.command, Command::Interrupt) {
            return Err(String::from("interrupt not allowed by audit"));
        }
        Ok(())
    }));
    let graph = build_interrupt_graph();

    let err = runner
        .invoke_thread_with_runtime_context(
            &graph,
            "thread-8",
            BTreeMap::new(),
            RuntimeContext::from([(String::from("marker"), json!("blocked"))]),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        CheckpointBridgeError::CommandAuditRejected { thread_id, .. } if thread_id == "thread-8"
    ));

    let checkpoints = saver.list("thread-8").expect("list should work");
    assert!(checkpoints.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn async_checkpoint_bridge_invocation_and_resume_work() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let first = runner
        .ainvoke_thread(&graph, "thread-async-1", BTreeMap::new())
        .await
        .expect("async invoke should succeed");
    let resumed = runner
        .aresume_latest(&graph, "thread-async-1")
        .await
        .expect("async resume_latest should succeed");

    assert_eq!(first.execution.state.get("events"), Some(&json!(["tick"])));
    assert_eq!(resumed.execution.state.get("events"), Some(&json!(["tick", "tick"])));
    assert_eq!(first.execution.metadata.versions_seen.get("events"), Some(&1));
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
}

#[tokio::test(flavor = "current_thread")]
async fn async_stream_thread_persists_checkpoint_with_events() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let graph = build_topic_round_graph();

    let streamed = runner
        .astream_thread(&graph, "thread-async-stream-1", BTreeMap::new())
        .await
        .expect("astream_thread should succeed");

    assert_eq!(streamed.execution.state.get("events"), Some(&json!(["tick"])));
    assert!(streamed.execution.interrupt.is_none());
    let has_start_event = streamed.execution.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::NodeStarted {
                node,
                superstep
            } if node == "start" && *superstep == 1
        )
    });
    assert!(has_start_event);

    let checkpoint = saver
        .get("thread-async-stream-1", &streamed.checkpoint_id)
        .expect("lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(checkpoint.versions_seen.get("events"), Some(&1));
    assert!(checkpoint.pending_writes.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn async_stream_interrupt_persists_checkpoint_and_supports_resume() {
    let saver = InMemorySaver::new();
    let runner = CheckpointedSequentialExecutor::new(&saver);
    let interrupt_graph = build_interrupt_graph();

    let runtime_context = RuntimeContext::from([(String::from("marker"), json!("stream-halt"))]);
    let streamed = runner
        .astream_thread_with_runtime_context(
            &interrupt_graph,
            "thread-async-stream-2",
            BTreeMap::new(),
            runtime_context,
        )
        .await
        .expect("astream interrupt should persist checkpoint");

    assert_eq!(
        streamed.execution.interrupt.as_ref().map(|signal| signal.node.as_str()),
        Some("gate")
    );
    let has_interrupt_event = streamed.execution.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::Interrupted {
                node,
                superstep
            } if node == "gate" && *superstep == 1
        )
    });
    assert!(has_interrupt_event);

    let checkpoint = saver
        .get("thread-async-stream-2", &streamed.checkpoint_id)
        .expect("lookup should succeed")
        .expect("checkpoint should exist");
    assert_eq!(checkpoint.state.get("events"), Some(&json!(["stream-halt"])));
    assert_eq!(checkpoint.versions_seen.get("events"), Some(&1));

    let resumed = runner
        .aresume_latest(&build_topic_round_graph(), "thread-async-stream-2")
        .await
        .expect("resume_latest should consume streamed checkpoint");
    assert_eq!(
        resumed.execution.state.get("events"),
        Some(&json!(["stream-halt", "tick"]))
    );
    assert_eq!(resumed.execution.metadata.versions_seen.get("events"), Some(&2));
}
