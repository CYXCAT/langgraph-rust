use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::RuntimeContext;
use langgraph_prebuilt::{
    build_default_tool_calling_agent, DefaultToolCallingAgentBuilder, ToolNode, ValidationNode,
    ValidationRule,
};
use langgraph_pregel::SequentialExecutor;
use serde_json::{json, Value};

#[test]
fn default_tool_calling_template_runs_single_tool_and_emits_final_answer() {
    let mut tool_node = ToolNode::new();
    tool_node.add_tool(
        "echo",
        Arc::new(|input, _runtime_context| Ok(json!({ "answer": input.get("query").cloned() }))),
    );

    let graph = build_default_tool_calling_agent("echo", tool_node).unwrap();
    let state = BTreeMap::from([(String::from("question"), json!("hello"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, state, RuntimeContext::new())
        .unwrap();

    assert_eq!(result.state.get("final_answer"), Some(&json!({ "answer": Some(json!("hello")) })));
    assert_eq!(result.state.get("tool_call"), Some(&json!(null)));
}

#[test]
fn default_tool_calling_template_supports_custom_keys_and_output_mapping() {
    let mut tool_node = ToolNode::new();
    tool_node.add_tool(
        "search",
        Arc::new(|input, _runtime_context| {
            Ok(json!({
                "doc": format!(
                    "doc:{}",
                    input
                        .get("q")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                )
            }))
        }),
    );

    let graph = DefaultToolCallingAgentBuilder::new("search", tool_node)
        .with_question_key("prompt")
        .with_tool_input_field("q")
        .with_final_answer_key("reply")
        .with_tool_input_builder(Arc::new(|state| {
            json!(format!(
                "{}::normalized",
                state.get("prompt").and_then(serde_json::Value::as_str).unwrap_or_default()
            ))
        }))
        .with_final_answer_builder(Arc::new(|tool_result, _state| {
            json!({
                "text": tool_result.get("doc").cloned().unwrap_or(Value::Null)
            })
        }))
        .build()
        .unwrap();

    let state = BTreeMap::from([(String::from("prompt"), json!("rust"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, state, RuntimeContext::new())
        .unwrap();

    assert_eq!(result.state.get("reply"), Some(&json!({ "text": "doc:rust::normalized" })));
}

#[test]
fn default_tool_calling_template_can_short_circuit_with_validator() {
    let mut validator = ValidationNode::new();
    validator.add_rule(ValidationRule::required("question"));

    let mut tool_node = ToolNode::new();
    tool_node.add_tool("echo", Arc::new(|input, _runtime_context| Ok(input.clone())));

    let graph = DefaultToolCallingAgentBuilder::new("echo", tool_node)
        .with_validator(validator)
        .build()
        .unwrap();

    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, BTreeMap::new(), RuntimeContext::new())
        .unwrap();

    assert_eq!(
        result.state.get("validation_errors"),
        Some(&json!(["field `question` is required"]))
    );
    assert_eq!(result.state.get("final_answer"), None);
}

#[test]
fn default_tool_calling_template_supports_tool_selection_strategy() {
    let mut tool_node = ToolNode::new();
    tool_node.add_tool(
        "search",
        Arc::new(|input, _runtime_context| {
            Ok(json!({
                "kind": "search",
                "answer": format!(
                    "search:{}",
                    input
                        .get("query")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                )
            }))
        }),
    );
    tool_node.add_tool(
        "math",
        Arc::new(|input, _runtime_context| {
            Ok(json!({
                "kind": "math",
                "answer": format!(
                    "math:{}",
                    input
                        .get("query")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                )
            }))
        }),
    );

    let graph = DefaultToolCallingAgentBuilder::new("search", tool_node)
        .with_tool_selector(Arc::new(|state| {
            let question = state.get("question").and_then(serde_json::Value::as_str)?;
            if question.contains("calculate") {
                Some(String::from("math"))
            } else {
                Some(String::from("search"))
            }
        }))
        .build()
        .unwrap();

    let state = BTreeMap::from([(String::from("question"), json!("calculate 1+1"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, state, RuntimeContext::new())
        .unwrap();

    assert_eq!(
        result.state.get("final_answer"),
        Some(&json!({ "kind": "math", "answer": "math:calculate 1+1" }))
    );
}

#[test]
fn default_tool_calling_template_supports_fallback_tool_for_unknown_selection() {
    let mut tool_node = ToolNode::new();
    tool_node.add_tool("search", Arc::new(|input, _runtime_context| Ok(input.clone())));
    tool_node.add_tool(
        "fallback",
        Arc::new(|input, _runtime_context| {
            Ok(json!({ "fallback_used": input.get("query").cloned() }))
        }),
    );

    let graph = DefaultToolCallingAgentBuilder::new("search", tool_node)
        .with_tool_selector(Arc::new(|_state| Some(String::from("non_existing_tool"))))
        .with_fallback_tool("fallback")
        .build()
        .unwrap();

    let state = BTreeMap::from([(String::from("question"), json!("hello fallback"))]);
    let result = SequentialExecutor
        .invoke_with_runtime_context(&graph, state, RuntimeContext::new())
        .unwrap();

    assert_eq!(
        result.state.get("final_answer"),
        Some(&json!({ "fallback_used": Some(json!("hello fallback")) }))
    );
}
