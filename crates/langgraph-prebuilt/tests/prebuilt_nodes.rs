use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{RuntimeContext, StateGraph};
use langgraph_prebuilt::{
    CompositeValidator, RuleSetValidator, ToolErrorStrategy, ToolNode, ToolRoutingStrategy,
    ValidationIssue, ValidationNode, ValidationRule,
};
use langgraph_pregel::SequentialExecutor;
use serde_json::json;

#[test]
fn tool_node_invokes_registered_tool_and_writes_result() {
    let mut graph = StateGraph::new();
    let mut tool_node = ToolNode::new();
    tool_node.add_tool(
        "echo_tenant",
        Arc::new(|input, runtime_context| {
            let tenant = runtime_context
                .get("tenant")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            Ok(json!({
                "tenant": tenant,
                "input": input.clone(),
            }))
        }),
    );

    graph.add_node_with_runtime("tool", tool_node.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("tool", "finish").unwrap();
    graph.set_entry_point("tool");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(
        String::from("tool_call"),
        json!({
            "name": "echo_tenant",
            "input": { "q": "hello" }
        }),
    )]);
    let context = RuntimeContext::from([(String::from("tenant"), json!("acme"))]);
    let result = SequentialExecutor.invoke_with_runtime_context(&compiled, state, context).unwrap();

    assert_eq!(
        result.state.get("tool_result"),
        Some(&json!({
            "tenant": "acme",
            "input": { "q": "hello" }
        }))
    );
    assert_eq!(result.state.get("tool_error"), Some(&json!(null)));
}

#[test]
fn tool_node_reports_unknown_tool_without_crashing_graph() {
    let mut graph = StateGraph::new();
    graph.add_node_with_runtime("tool", ToolNode::new().into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("tool", "finish").unwrap();
    graph.set_entry_point("tool");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(
        String::from("tool_call"),
        json!({
            "name": "not_found",
            "input": {}
        }),
    )]);
    let result = SequentialExecutor.invoke(&compiled, state).unwrap();

    assert_eq!(result.get("tool_result"), None);
    assert_eq!(result.get("tool_error"), Some(&json!("tool `not_found` is not registered")));
}

#[test]
fn validation_node_collects_required_and_custom_rule_errors() {
    let mut graph = StateGraph::new();
    let mut validation = ValidationNode::new();
    validation.add_rule(ValidationRule::required("user_id"));
    validation.add_rule(ValidationRule::string_min_len("query", 3));
    validation.add_rule(ValidationRule::new(
        "priority",
        Arc::new(|value| {
            if value.and_then(serde_json::Value::as_i64).unwrap_or_default() > 0 {
                Ok(())
            } else {
                Err(ValidationIssue::new(
                    Some("priority"),
                    "range_error",
                    "field `priority` must be > 0",
                ))
            }
        }),
    ));

    graph.add_node_with_runtime("validate", validation.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("validate", "finish").unwrap();
    graph.set_entry_point("validate");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([
        (String::from("query"), json!("hi")),
        (String::from("priority"), json!(0)),
    ]);
    let result = SequentialExecutor.invoke(&compiled, state).unwrap();

    assert_eq!(
        result.get("validation_errors"),
        Some(&json!([
            "field `user_id` is required",
            "field `query` must be at least 3 characters",
            "field `priority` must be > 0"
        ]))
    );
    assert_eq!(
        result.get("validation_report"),
        Some(&json!({
            "valid": false,
            "issues": [
                {
                    "field": "user_id",
                    "code": "required",
                    "message": "field `user_id` is required"
                },
                {
                    "field": "query",
                    "code": "min_length",
                    "message": "field `query` must be at least 3 characters"
                },
                {
                    "field": "priority",
                    "code": "range_error",
                    "message": "field `priority` must be > 0"
                }
            ]
        }))
    );
}

#[test]
fn tool_node_fallback_routing_uses_default_tool_for_unknown_name() {
    let mut graph = StateGraph::new();
    let mut tool_node = ToolNode::new()
        .with_routing_strategy(ToolRoutingStrategy::FallbackToDefault)
        .with_default_tool("echo");
    tool_node.add_tool("echo", Arc::new(|input, _context| Ok(input.clone())));

    graph.add_node_with_runtime("tool", tool_node.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("tool", "finish").unwrap();
    graph.set_entry_point("tool");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(
        String::from("tool_call"),
        json!({
            "name": "missing_tool",
            "input": { "value": 42 }
        }),
    )]);
    let result = SequentialExecutor.invoke(&compiled, state).unwrap();

    assert_eq!(result.get("tool_result"), Some(&json!({ "value": 42 })));
    assert_eq!(result.get("tool_error"), Some(&json!(null)));
}

#[test]
fn tool_node_can_reject_with_node_error_strategy() {
    let mut graph = StateGraph::new();
    let tool_node = ToolNode::new().with_error_strategy(ToolErrorStrategy::ReturnNodeError);

    graph.add_node_with_runtime("tool", tool_node.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("tool", "finish").unwrap();
    graph.set_entry_point("tool");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(
        String::from("tool_call"),
        json!({
            "name": "missing_tool",
            "input": {}
        }),
    )]);
    let error = SequentialExecutor.invoke(&compiled, state).unwrap_err();
    assert!(format!("{error}").contains("tool `missing_tool` is not registered"));
}

#[test]
fn tool_node_audit_hook_can_reject_invocation() {
    let mut graph = StateGraph::new();
    let mut tool_node = ToolNode::new().with_audit_hook(Arc::new(|invocation, _context| {
        if invocation.requested_tool == "blocked" {
            return Err(String::from("blocked by policy"));
        }
        Ok(())
    }));
    tool_node.add_tool("echo", Arc::new(|input, _context| Ok(input.clone())));
    tool_node.add_alias("blocked", "echo");

    graph.add_node_with_runtime("tool", tool_node.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("tool", "finish").unwrap();
    graph.set_entry_point("tool");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(
        String::from("tool_call"),
        json!({
            "name": "blocked",
            "input": {}
        }),
    )]);
    let result = SequentialExecutor.invoke(&compiled, state).unwrap();
    assert_eq!(
        result.get("tool_error"),
        Some(&json!("tool audit rejected invocation: blocked by policy"))
    );
}

#[test]
fn validation_node_supports_composite_any_validator() {
    let mut graph = StateGraph::new();
    let mut validation = ValidationNode::new();

    let mut email_rules = RuleSetValidator::new();
    email_rules.add_rule(ValidationRule::required("email"));

    let mut phone_rules = RuleSetValidator::new();
    phone_rules.add_rule(ValidationRule::required("phone"));

    let any_contact = CompositeValidator::any(vec![email_rules.into_ref(), phone_rules.into_ref()]);
    validation.add_validator(any_contact.into_ref());

    graph.add_node_with_runtime("validate", validation.into_action()).unwrap();
    graph.add_node("finish", Arc::new(|_state| Ok(BTreeMap::new()))).unwrap();
    graph.add_edge("validate", "finish").unwrap();
    graph.set_entry_point("validate");
    graph.set_finish_point("finish");
    let compiled = graph.compile().unwrap();

    let state = BTreeMap::from([(String::from("phone"), json!("18812345678"))]);
    let result = SequentialExecutor.invoke(&compiled, state).unwrap();

    assert_eq!(result.get("validation_errors"), Some(&json!([])));
    assert_eq!(
        result.get("validation_report"),
        Some(&json!({
            "valid": true,
            "issues": []
        }))
    );
}
