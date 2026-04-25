use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use langgraph_checkpoint::{Checkpoint, CheckpointError, CheckpointSaver};
use langgraph_checkpoint_postgres::PostgresSaver;
use langgraph_checkpoint_sqlite::SqliteSaver;
use langgraph_core::{NodeOutput, RuntimeContext, StateGraph, Topic};
use langgraph_pregel::{
    CheckpointBridgeError, CheckpointedSequentialExecutor, ThreadInvocationOutcome,
};
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

fn postgres_test_url() -> Option<String> {
    env::var("LANGGRAPH_POSTGRES_TEST_URL").ok()
}

fn unique_thread_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    format!("{prefix}-{nanos}")
}

fn make_checkpoint(thread_id: String, checkpoint_id: &str) -> Checkpoint {
    Checkpoint {
        thread_id,
        checkpoint_id: checkpoint_id.to_string(),
        state: BTreeMap::new(),
        versions_seen: BTreeMap::new(),
        pending_writes: Vec::new(),
    }
}

#[test]
fn sqlite_and_postgres_match_on_invoke_and_resume_latest() {
    let Some(pg_url) = postgres_test_url() else {
        return;
    };
    let graph = build_topic_round_graph();
    let sqlite_dir = tempdir().expect("tempdir should be created");
    let sqlite_path = sqlite_dir.path().join("parity.db");
    let sqlite_saver = Arc::new(SqliteSaver::new(&sqlite_path).expect("sqlite saver should init"));
    let postgres_saver = Arc::new(PostgresSaver::new(pg_url).expect("postgres saver should init"));

    let sqlite_runner = CheckpointedSequentialExecutor::new(sqlite_saver);
    let postgres_runner = CheckpointedSequentialExecutor::new(postgres_saver);
    let sqlite_thread = unique_thread_id("thread-parity-invoke-sqlite");
    let postgres_thread = unique_thread_id("thread-parity-invoke-postgres");

    let sqlite_first = sqlite_runner
        .invoke_thread(&graph, sqlite_thread.clone(), BTreeMap::new())
        .expect("sqlite first invoke should succeed");
    let pg_first = postgres_runner
        .invoke_thread(&graph, postgres_thread.clone(), BTreeMap::new())
        .expect("postgres first invoke should succeed");
    assert_eq!(sqlite_first.execution.state, pg_first.execution.state);
    assert_eq!(
        sqlite_first.execution.metadata.versions_seen,
        pg_first.execution.metadata.versions_seen
    );

    let sqlite_resumed = sqlite_runner
        .resume_latest(&graph, sqlite_thread)
        .expect("sqlite resume_latest should succeed");
    let pg_resumed = postgres_runner
        .resume_latest(&graph, postgres_thread)
        .expect("postgres resume_latest should succeed");

    assert_eq!(sqlite_resumed.execution.state, pg_resumed.execution.state);
    assert_eq!(
        sqlite_resumed.execution.metadata.versions_seen,
        pg_resumed.execution.metadata.versions_seen
    );
}

#[test]
fn sqlite_and_postgres_match_on_interrupt_and_resume_behavior() {
    let Some(pg_url) = postgres_test_url() else {
        return;
    };
    let interrupt_graph = build_interrupt_graph();
    let resume_graph = build_topic_round_graph();
    let sqlite_dir = tempdir().expect("tempdir should be created");
    let sqlite_path = sqlite_dir.path().join("parity.db");
    let sqlite_saver = Arc::new(SqliteSaver::new(&sqlite_path).expect("sqlite saver should init"));
    let postgres_saver = Arc::new(PostgresSaver::new(pg_url).expect("postgres saver should init"));

    let sqlite_runner = CheckpointedSequentialExecutor::new(sqlite_saver);
    let postgres_runner = CheckpointedSequentialExecutor::new(postgres_saver);
    let sqlite_thread = unique_thread_id("thread-parity-interrupt-sqlite");
    let postgres_thread = unique_thread_id("thread-parity-interrupt-postgres");
    let runtime_context = RuntimeContext::from([(String::from("marker"), json!("halt"))]);

    let sqlite_outcome = sqlite_runner
        .invoke_thread_with_runtime_context(
            &interrupt_graph,
            sqlite_thread.clone(),
            BTreeMap::new(),
            runtime_context.clone(),
        )
        .expect("sqlite interrupt invoke should succeed");
    let pg_outcome = postgres_runner
        .invoke_thread_with_runtime_context(
            &interrupt_graph,
            postgres_thread.clone(),
            BTreeMap::new(),
            runtime_context,
        )
        .expect("postgres interrupt invoke should succeed");

    let sqlite_interrupted = match sqlite_outcome {
        ThreadInvocationOutcome::Interrupted(interrupted) => interrupted,
        ThreadInvocationOutcome::Completed(_) => panic!("sqlite expected interrupted outcome"),
    };
    let pg_interrupted = match pg_outcome {
        ThreadInvocationOutcome::Interrupted(interrupted) => interrupted,
        ThreadInvocationOutcome::Completed(_) => panic!("postgres expected interrupted outcome"),
    };
    assert_eq!(sqlite_interrupted.interrupt.node, pg_interrupted.interrupt.node);
    assert_eq!(sqlite_interrupted.interrupt.superstep, pg_interrupted.interrupt.superstep);

    let sqlite_resumed = sqlite_runner
        .resume_latest(&resume_graph, sqlite_thread)
        .expect("sqlite resume_latest should succeed");
    let pg_resumed = postgres_runner
        .resume_latest(&resume_graph, postgres_thread)
        .expect("postgres resume_latest should succeed");

    assert_eq!(sqlite_resumed.execution.state, pg_resumed.execution.state);
    assert_eq!(
        sqlite_resumed.execution.metadata.versions_seen,
        pg_resumed.execution.metadata.versions_seen
    );
}

#[test]
fn sqlite_and_postgres_match_on_duplicate_checkpoint_conflict() {
    let Some(pg_url) = postgres_test_url() else {
        return;
    };
    let sqlite_dir = tempdir().expect("tempdir should be created");
    let sqlite_path = sqlite_dir.path().join("parity.db");
    let sqlite_saver = SqliteSaver::new(&sqlite_path).expect("sqlite saver should init");
    let postgres_saver = PostgresSaver::new(pg_url).expect("postgres saver should init");
    let sqlite_thread = unique_thread_id("thread-parity-conflict-sqlite");
    let postgres_thread = unique_thread_id("thread-parity-conflict-postgres");
    let checkpoint_id = "cp-dup";

    sqlite_saver
        .put(make_checkpoint(sqlite_thread.clone(), checkpoint_id))
        .expect("sqlite first put should succeed");
    postgres_saver
        .put(make_checkpoint(postgres_thread.clone(), checkpoint_id))
        .expect("postgres first put should succeed");

    let sqlite_err = sqlite_saver
        .put(make_checkpoint(sqlite_thread, checkpoint_id))
        .expect_err("sqlite duplicate put should fail");
    let pg_err = postgres_saver
        .put(make_checkpoint(postgres_thread, checkpoint_id))
        .expect_err("postgres duplicate put should fail");

    assert!(matches!(sqlite_err, CheckpointError::Conflict(_)));
    assert!(matches!(pg_err, CheckpointError::Conflict(_)));
}

#[test]
fn sqlite_and_postgres_match_on_resume_missing_checkpoint_error() {
    let Some(pg_url) = postgres_test_url() else {
        return;
    };
    let graph = build_topic_round_graph();
    let sqlite_dir = tempdir().expect("tempdir should be created");
    let sqlite_path = sqlite_dir.path().join("parity.db");
    let sqlite_saver = Arc::new(SqliteSaver::new(&sqlite_path).expect("sqlite saver should init"));
    let postgres_saver = Arc::new(PostgresSaver::new(pg_url).expect("postgres saver should init"));

    let sqlite_runner = CheckpointedSequentialExecutor::new(sqlite_saver);
    let postgres_runner = CheckpointedSequentialExecutor::new(postgres_saver);
    let sqlite_thread = unique_thread_id("thread-parity-missing-sqlite");
    let postgres_thread = unique_thread_id("thread-parity-missing-postgres");
    let checkpoint_id = "cp-missing";

    let sqlite_err = sqlite_runner
        .resume(&graph, sqlite_thread.clone(), checkpoint_id)
        .expect_err("sqlite missing checkpoint should error");
    let pg_err = postgres_runner
        .resume(&graph, postgres_thread.clone(), checkpoint_id)
        .expect_err("postgres missing checkpoint should error");

    assert!(matches!(
        sqlite_err,
        CheckpointBridgeError::CheckpointNotFound {
            thread_id,
            checkpoint_id: missing
        } if thread_id == sqlite_thread && missing == checkpoint_id
    ));
    assert!(matches!(
        pg_err,
        CheckpointBridgeError::CheckpointNotFound {
            thread_id,
            checkpoint_id: missing
        } if thread_id == postgres_thread && missing == checkpoint_id
    ));
}

#[test]
fn sqlite_and_postgres_match_on_resume_latest_empty_thread_error() {
    let Some(pg_url) = postgres_test_url() else {
        return;
    };
    let graph = build_topic_round_graph();
    let sqlite_dir = tempdir().expect("tempdir should be created");
    let sqlite_path = sqlite_dir.path().join("parity.db");
    let sqlite_saver = Arc::new(SqliteSaver::new(&sqlite_path).expect("sqlite saver should init"));
    let postgres_saver = Arc::new(PostgresSaver::new(pg_url).expect("postgres saver should init"));

    let sqlite_runner = CheckpointedSequentialExecutor::new(sqlite_saver);
    let postgres_runner = CheckpointedSequentialExecutor::new(postgres_saver);
    let sqlite_thread = unique_thread_id("thread-parity-empty-sqlite");
    let postgres_thread = unique_thread_id("thread-parity-empty-postgres");

    let sqlite_err = sqlite_runner
        .resume_latest(&graph, sqlite_thread.clone())
        .expect_err("sqlite empty thread should error");
    let pg_err = postgres_runner
        .resume_latest(&graph, postgres_thread.clone())
        .expect_err("postgres empty thread should error");

    assert!(matches!(
        sqlite_err,
        CheckpointBridgeError::EmptyThread(thread_id) if thread_id == sqlite_thread
    ));
    assert!(matches!(
        pg_err,
        CheckpointBridgeError::EmptyThread(thread_id) if thread_id == postgres_thread
    ));
}
