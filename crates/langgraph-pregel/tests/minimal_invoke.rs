use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{
    BinaryOperatorAggregate, Command, GraphError, LastValue, NodeOutput, RuntimeContext, StateGraph,
};
use langgraph_pregel::{CommandPolicy, SequentialExecutor, StreamEvent};
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
fn compile_rejects_finish_with_outgoing_edges() {
    let mut graph = StateGraph::new();
    graph.add_node("start", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("tail", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("start", "finish").unwrap();
    graph.add_edge("finish", "tail").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");

    assert!(matches!(
        graph.compile(),
        Err(GraphError::FinishHasOutgoingEdges(node)) if node == "finish"
    ));
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
fn invoke_last_value_channel_rejects_multiple_writes_in_same_superstep() {
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
    let err = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap_err();
    assert!(matches!(err, GraphError::ChannelMergeFailed { field, .. } if field == "status"));
}

#[test]
fn invoke_does_not_exit_early_when_finish_is_parallel_with_other_work() {
    let mut graph = StateGraph::new();
    graph.add_node("start", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph
        .add_node(
            "slow_1",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("trail"), json!("slow_1"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "slow_2",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("trail"), json!("slow_2"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "finish",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("done"), json!(true))]))),
        )
        .unwrap();

    graph.add_edge("start", "finish").unwrap();
    graph.add_edge("start", "slow_1").unwrap();
    graph.add_edge("slow_1", "slow_2").unwrap();
    graph.add_edge("slow_2", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("trail", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap();
    assert_eq!(final_state.get("done"), Some(&json!(true)));
    assert_eq!(final_state.get("trail"), Some(&json!(["slow_1", "slow_2"])));
}

#[test]
fn invoke_tracks_channel_versions_across_supersteps() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "start",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("seed"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "middle",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("middle"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("start", "middle").unwrap();
    graph.add_edge("middle", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor.invoke_with_metadata(&compiled, BTreeMap::new()).unwrap();

    assert_eq!(result.state.get("events"), Some(&json!(["seed", "middle"])));
    assert_eq!(result.metadata.channel_versions.get("events"), Some(&2));
    assert_eq!(result.metadata.versions_seen.get("events"), Some(&2));
    assert_eq!(result.metadata.supersteps, 3);
}

#[test]
fn invoke_metadata_tracks_only_channel_fields() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "start",
            Arc::new(|_state| {
                Ok(BTreeMap::from([
                    (String::from("events"), json!("seed")),
                    (String::from("note"), json!("v1")),
                ]))
            }),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("start", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor.invoke_with_metadata(&compiled, BTreeMap::new()).unwrap();

    assert_eq!(result.metadata.channel_versions.get("events"), Some(&1));
    assert_eq!(result.metadata.versions_seen.get("events"), Some(&1));
    assert_eq!(result.metadata.channel_versions.get("note"), None);
    assert_eq!(result.metadata.versions_seen.get("note"), None);
}

#[test]
fn topic_channel_appends_across_rounds_with_snapshot_consistency() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "start",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("seed"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "branch_a",
            Arc::new(|state| {
                let seen = state
                    .get("events")
                    .and_then(|value| value.as_array())
                    .map_or(0, std::vec::Vec::len);
                Ok(BTreeMap::from([
                    (String::from("events"), json!("a")),
                    (String::from("seen_by_a"), json!(seen)),
                ]))
            }),
        )
        .unwrap();
    graph
        .add_node(
            "branch_b",
            Arc::new(|state| {
                let seen = state
                    .get("events")
                    .and_then(|value| value.as_array())
                    .map_or(0, std::vec::Vec::len);
                Ok(BTreeMap::from([
                    (String::from("events"), json!("b")),
                    (String::from("seen_by_b"), json!(seen)),
                ]))
            }),
        )
        .unwrap();
    graph
        .add_node(
            "collector",
            Arc::new(|state| {
                let seen = state
                    .get("events")
                    .and_then(|value| value.as_array())
                    .map_or(0, std::vec::Vec::len);
                Ok(BTreeMap::from([
                    (String::from("events"), json!("c")),
                    (String::from("seen_by_collector"), json!(seen)),
                ]))
            }),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();

    graph.add_edge("start", "branch_a").unwrap();
    graph.add_edge("start", "branch_b").unwrap();
    graph.add_edge("branch_a", "collector").unwrap();
    graph.add_edge("branch_b", "collector").unwrap();
    graph.add_edge("collector", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap();

    assert_eq!(final_state.get("events"), Some(&json!(["seed", "a", "b", "c"])));
    assert_eq!(final_state.get("seen_by_a"), Some(&json!(1)));
    assert_eq!(final_state.get("seen_by_b"), Some(&json!(1)));
    assert_eq!(final_state.get("seen_by_collector"), Some(&json!(3)));
}

#[test]
fn invoke_with_runtime_context_injects_read_only_values_to_nodes() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "start",
            Arc::new(|_state, runtime_context| {
                let tenant = runtime_context
                    .get("tenant")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                Ok(NodeOutput::from_patch(BTreeMap::from([(
                    String::from("tenant"),
                    json!(tenant),
                )])))
            }),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("start", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let runtime_context = RuntimeContext::from([(String::from("tenant"), json!("acme"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&compiled, BTreeMap::new(), runtime_context)
        .unwrap();

    assert_eq!(result.interrupt, None);
    assert_eq!(result.state.get("tenant"), Some(&json!("acme")));
}

#[test]
fn invoke_with_runtime_context_supports_interrupt_command() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "gate",
            Arc::new(|_state, _context| {
                Ok(NodeOutput::interrupt(BTreeMap::from([(
                    String::from("status"),
                    json!("paused"),
                )])))
            }),
        )
        .unwrap();
    graph.set_entry_point("gate");
    graph.set_finish_point("gate");

    let compiled = graph.compile().unwrap();
    let result =
        SequentialExecutor.invoke_with_runtime_context(&compiled, BTreeMap::new(), BTreeMap::new()).unwrap();

    assert_eq!(result.state.get("status"), Some(&json!("paused")));
    assert_eq!(result.interrupt.as_ref().map(|signal| signal.node.as_str()), Some("gate"));
    assert_eq!(result.interrupt.as_ref().map(|signal| signal.superstep), Some(1));

    let interrupted = SequentialExecutor.invoke_with_metadata(&compiled, BTreeMap::new()).unwrap_err();
    assert!(matches!(interrupted, GraphError::Interrupted { node } if node == "gate"));
}

#[test]
fn invoke_supports_conditional_edges_routing() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "router",
            Arc::new(|state| {
                let route = state
                    .get("route")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("left");
                Ok(BTreeMap::from([(String::from("route"), json!(route))]))
            }),
        )
        .unwrap();
    graph
        .add_node(
            "left",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("picked"), json!("L"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "right",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("picked"), json!("R"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_edge("right", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left", "right"],
        Arc::new(|state, _context| {
            let route = state
                .get("route")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("left");
            Ok(vec![route.to_string()])
        }),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let initial = BTreeMap::from([(String::from("route"), json!("right"))]);
    let final_state = SequentialExecutor.invoke(&compiled, initial).unwrap();
    assert_eq!(final_state.get("picked"), Some(&json!("R")));
}

#[test]
fn invoke_command_goto_overrides_conditional_route() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| Ok(NodeOutput::goto(BTreeMap::new(), "right"))),
        )
        .unwrap();
    graph
        .add_node(
            "left",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("picked"), json!("L"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "right",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("picked"), json!("R"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_edge("right", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left", "right"],
        Arc::new(|_state, _context| Ok(vec![String::from("left")])),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let final_state = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap();
    assert_eq!(final_state.get("picked"), Some(&json!("R")));
}

#[test]
fn invoke_rejects_invalid_conditional_target_from_router() {
    let mut graph = StateGraph::new();
    graph.add_node("router", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("left", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left"],
        Arc::new(|_state, _context| Ok(vec![String::from("ghost")])),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let err = SequentialExecutor.invoke(&compiled, BTreeMap::new()).unwrap_err();
    assert!(matches!(
        err,
        GraphError::InvalidConditionalTarget { from, to } if from == "router" && to == "ghost"
    ));
}

#[test]
fn invoke_supports_goto_many_targets() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| Ok(NodeOutput::goto_many(BTreeMap::new(), ["left", "right"]))),
        )
        .unwrap();
    graph
        .add_node(
            "left",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("marks"), json!("L"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "right",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("marks"), json!("R"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_edge("right", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left", "right"],
        Arc::new(|_state, _context| Ok(Vec::new())),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");
    graph.add_channel("marks", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor
        .invoke_with_runtime_context(&compiled, BTreeMap::new(), BTreeMap::new())
        .unwrap();
    assert_eq!(result.state.get("marks"), Some(&json!(["L", "R"])));
}

#[test]
fn invoke_records_command_trace_for_composed_commands() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| {
                Ok(NodeOutput::from_patch(BTreeMap::new()).with_commands(vec![
                    Command::Goto(String::from("left")),
                    Command::GotoMany(vec![String::from("right")]),
                ]))
            }),
        )
        .unwrap();
    graph.add_node("left", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("right", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_edge("right", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left", "right"],
        Arc::new(|_state, _context| Ok(Vec::new())),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor
        .invoke_with_runtime_context(&compiled, BTreeMap::new(), BTreeMap::new())
        .unwrap();
    assert_eq!(result.interrupt, None);
    assert_eq!(result.metadata.command_trace.len(), 2);
    assert_eq!(result.metadata.command_trace[0].node, "router");
    assert_eq!(result.metadata.command_trace[0].superstep, 1);
    assert!(matches!(
        result.metadata.command_trace[0].command,
        Command::Goto(ref target) if target == "left"
    ));
    assert!(matches!(
        result.metadata.command_trace[1].command,
        Command::GotoMany(ref targets) if targets == &vec![String::from("right")]
    ));
}

#[test]
fn invoke_rejects_goto_many_when_policy_disables_it() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| Ok(NodeOutput::goto_many(BTreeMap::new(), ["left", "right"]))),
        )
        .unwrap();
    graph.add_node("left", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("right", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("left", "finish").unwrap();
    graph.add_edge("right", "finish").unwrap();
    graph.add_conditional_edges(
        "router",
        ["left", "right"],
        Arc::new(|_state, _context| Ok(Vec::new())),
    );
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let policy = CommandPolicy {
        allow_goto_many: false,
        ..CommandPolicy::default()
    };
    let err = SequentialExecutor
        .invoke_with_runtime_context_and_policy(
            &compiled,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            &policy,
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::CommandRejected { node, .. } if node == "router"
    ));
}

#[test]
fn invoke_rejects_mixed_interrupt_and_routing_when_policy_forbids() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| {
                Ok(NodeOutput::from_patch(BTreeMap::new()).with_commands(vec![
                    Command::Interrupt,
                    Command::Goto(String::from("finish")),
                ]))
            }),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_conditional_edges("router", ["finish"], Arc::new(|_state, _context| Ok(Vec::new())));
    graph.set_entry_point("router");
    graph.set_finish_point("finish");

    let compiled = graph.compile().unwrap();
    let policy = CommandPolicy {
        allow_interrupt_with_routing: false,
        ..CommandPolicy::default()
    };
    let err = SequentialExecutor
        .invoke_with_runtime_context_and_policy(
            &compiled,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            &policy,
        )
        .unwrap_err();

    assert!(matches!(
        err,
        GraphError::CommandRejected { node, .. } if node == "router"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn ainvoke_matches_sync_invoke_state_and_metadata() {
    let mut graph = StateGraph::new();
    graph
        .add_node(
            "start",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("seed"))]))),
        )
        .unwrap();
    graph
        .add_node(
            "middle",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("events"), json!("middle"))]))),
        )
        .unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("start", "middle").unwrap();
    graph.add_edge("middle", "finish").unwrap();
    graph.set_entry_point("start");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let sync_result = SequentialExecutor.invoke_with_metadata(&compiled, BTreeMap::new()).unwrap();
    let async_result = SequentialExecutor
        .ainvoke_with_metadata(&compiled, BTreeMap::new())
        .await
        .unwrap();

    assert_eq!(async_result.state, sync_result.state);
    assert_eq!(async_result.metadata.supersteps, sync_result.metadata.supersteps);
    assert_eq!(
        async_result.metadata.versions_seen,
        sync_result.metadata.versions_seen
    );
}

#[tokio::test(flavor = "current_thread")]
async fn astream_emits_node_events_state_chunks_and_commands() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "router",
            Arc::new(|_state, _context| {
                Ok(NodeOutput::from_patch(BTreeMap::from([(String::from("events"), json!("seed"))]))
                    .with_commands(vec![Command::Goto(String::from("finish"))]))
            }),
        )
        .unwrap();
    graph
        .add_node(
            "finish",
            Arc::new(|_state| Ok(BTreeMap::from([(String::from("status"), json!("done"))]))),
        )
        .unwrap();
    graph.add_edge("router", "finish").unwrap();
    graph.set_entry_point("router");
    graph.set_finish_point("finish");
    graph.add_channel("events", Arc::new(langgraph_core::Topic));

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor.astream(&compiled, BTreeMap::new()).await.unwrap();

    assert_eq!(result.state.get("events"), Some(&json!(["seed"])));
    assert_eq!(result.state.get("status"), Some(&json!("done")));
    assert_eq!(result.interrupt, None);

    let has_router_start = result.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::NodeStarted {
                node,
                superstep
            } if node == "router" && *superstep == 1
        )
    });
    let has_router_finish = result.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::NodeFinished {
                node,
                superstep,
                ..
            } if node == "router" && *superstep == 1
        )
    });
    let has_command = result.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::CommandEmitted {
                node,
                superstep,
                command
            } if node == "router" && *superstep == 1 && matches!(command, Command::Goto(target) if target == "finish")
        )
    });
    let has_state_chunk = result.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::StateChunk {
                superstep,
                chunk
            } if *superstep == 1 && chunk.get("events") == Some(&json!(["seed"]))
        )
    });

    assert!(has_router_start);
    assert!(has_router_finish);
    assert!(has_command);
    assert!(has_state_chunk);
}

#[tokio::test(flavor = "current_thread")]
async fn astream_emits_interrupt_event_when_node_interrupts() {
    let mut graph = StateGraph::new();
    graph
        .add_node_with_runtime(
            "gate",
            Arc::new(|_state, _context| {
                Ok(NodeOutput::interrupt(BTreeMap::from([(
                    String::from("status"),
                    json!("paused"),
                )])))
            }),
        )
        .unwrap();
    graph.set_entry_point("gate");
    graph.set_finish_point("gate");

    let compiled = graph.compile().unwrap();
    let result = SequentialExecutor.astream(&compiled, BTreeMap::new()).await.unwrap();

    assert_eq!(result.interrupt.as_ref().map(|signal| signal.node.as_str()), Some("gate"));
    let has_interrupt = result.events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::Interrupted {
                node,
                superstep
            } if node == "gate" && *superstep == 1
        )
    });
    assert!(has_interrupt);
}
