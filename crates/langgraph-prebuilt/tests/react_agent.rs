use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{NodeOutput, RuntimeContext, RuntimeNodeAction};
use langgraph_prebuilt::{
    build_react_agent, ReactAgentBuilder, ToolNode, ValidationNode, ValidationRule,
};
use langgraph_pregel::SequentialExecutor;
use serde_json::{json, Value};

fn planner_action() -> RuntimeNodeAction {
    Arc::new(|state, _runtime_context| {
        if let Some(tool_result) = state.get("tool_result") {
            return Ok(NodeOutput::from_patch(BTreeMap::from([
                (String::from("final_answer"), tool_result.clone()),
                (String::from("tool_call"), json!(null)),
            ])));
        }

        let query = state.get("question").cloned().unwrap_or(json!(""));
        Ok(NodeOutput::from_patch(BTreeMap::from([(
            String::from("tool_call"),
            json!({
                "name": "echo",
                "input": { "query": query }
            }),
        )])))
    })
}

#[test]
fn react_agent_routes_between_planner_and_tool_until_finish() {
    let mut tool_node = ToolNode::new();
    tool_node.add_tool(
        "echo",
        Arc::new(|input, _runtime_context| {
            let query = input.get("query").cloned().unwrap_or(Value::Null);
            Ok(json!({ "answer": query }))
        }),
    );

    let graph = build_react_agent(planner_action(), tool_node).unwrap();
    let state = BTreeMap::from([(String::from("question"), json!("hello"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, state, RuntimeContext::new())
        .unwrap();

    assert_eq!(result.state.get("final_answer"), Some(&json!({ "answer": "hello" })));
    assert_eq!(result.state.get("tool_error"), Some(&json!(null)));
}

#[test]
fn react_agent_with_validator_stops_before_planner_when_invalid() {
    let mut validator = ValidationNode::new();
    validator.add_rule(ValidationRule::required("question"));

    let mut tool_node = ToolNode::new();
    tool_node.add_tool("echo", Arc::new(|input, _runtime_context| Ok(input.clone())));

    let graph = ReactAgentBuilder::new(planner_action(), tool_node)
        .with_validator(validator)
        .build()
        .unwrap();
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, BTreeMap::new(), RuntimeContext::new())
        .unwrap();

    assert_eq!(result.state.get("tool_call"), None);
    assert_eq!(result.state.get("final_answer"), None);
    assert_eq!(
        result.state.get("validation_errors"),
        Some(&json!(["field `question` is required"]))
    );
}
