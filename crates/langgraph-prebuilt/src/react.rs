use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{
    Command, CompiledGraph, GraphError, NodeAction, RuntimeNodeAction, State, StateGraph,
    StatePatch,
};

use crate::{ToolNode, ValidationNode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactNodeNames {
    pub validate: String,
    pub planner: String,
    pub tools: String,
    pub finish: String,
}

impl Default for ReactNodeNames {
    fn default() -> Self {
        Self {
            validate: String::from("validate"),
            planner: String::from("planner"),
            tools: String::from("tools"),
            finish: String::from("finish"),
        }
    }
}

pub struct ReactAgentBuilder {
    planner: RuntimeNodeAction,
    tool_node: ToolNode,
    validator: Option<ValidationNode>,
    tool_call_key: String,
    node_names: ReactNodeNames,
}

impl ReactAgentBuilder {
    #[must_use]
    pub fn new(planner: RuntimeNodeAction, tool_node: ToolNode) -> Self {
        Self {
            planner,
            tool_node,
            validator: None,
            tool_call_key: String::from("tool_call"),
            node_names: ReactNodeNames::default(),
        }
    }

    #[must_use]
    pub fn with_validator(mut self, validator: ValidationNode) -> Self {
        self.validator = Some(validator);
        self
    }

    #[must_use]
    pub fn with_tool_call_key(mut self, key: impl Into<String>) -> Self {
        self.tool_call_key = key.into();
        self
    }

    #[must_use]
    pub fn with_node_names(mut self, names: ReactNodeNames) -> Self {
        self.node_names = names;
        self
    }

    pub fn build(self) -> Result<CompiledGraph, GraphError> {
        let mut graph = StateGraph::new();

        graph.add_node_with_runtime(
            self.node_names.planner.clone(),
            planner_with_default_routing(
                self.planner,
                self.tool_call_key.clone(),
                self.node_names.tools.clone(),
                self.node_names.finish.clone(),
            ),
        )?;
        graph.add_node_with_runtime(self.node_names.tools.clone(), self.tool_node.into_action())?;
        graph.add_node(self.node_names.finish.clone(), noop_finish_node())?;

        graph.add_edge(self.node_names.tools.clone(), self.node_names.planner.clone())?;
        graph.add_conditional_edges(
            self.node_names.planner.clone(),
            [self.node_names.tools.clone(), self.node_names.finish.clone()],
            planner_router(
                self.tool_call_key.clone(),
                self.node_names.tools.clone(),
                self.node_names.finish.clone(),
            ),
        );

        let entry_point = if let Some(validator) = self.validator {
            let validation_key = validator.output_key().to_string();
            graph.add_node_with_runtime(
                self.node_names.validate.clone(),
                validation_with_default_routing(
                    validator,
                    validation_key.clone(),
                    self.node_names.planner.clone(),
                    self.node_names.finish.clone(),
                ),
            )?;
            graph.add_conditional_edges(
                self.node_names.validate.clone(),
                [self.node_names.planner.clone(), self.node_names.finish.clone()],
                validation_router(
                    validation_key,
                    self.node_names.planner.clone(),
                    self.node_names.finish.clone(),
                ),
            );
            self.node_names.validate
        } else {
            self.node_names.planner
        };

        graph.set_entry_point(entry_point);
        graph.set_finish_point(self.node_names.finish);
        graph.compile()
    }
}

pub fn build_react_agent(
    planner: RuntimeNodeAction,
    tool_node: ToolNode,
) -> Result<CompiledGraph, GraphError> {
    ReactAgentBuilder::new(planner, tool_node).build()
}

fn planner_router(
    tool_call_key: String,
    tools: String,
    finish: String,
) -> langgraph_core::ConditionalRouteFn {
    Arc::new(move |state, _runtime_context| {
        if has_tool_call(state, &tool_call_key) {
            return Ok(vec![tools.clone()]);
        }
        Ok(vec![finish.clone()])
    })
}

fn planner_with_default_routing(
    planner: RuntimeNodeAction,
    tool_call_key: String,
    tools: String,
    finish: String,
) -> RuntimeNodeAction {
    Arc::new(move |state, runtime_context| {
        let mut output = planner(state, runtime_context)?;
        if !output.has_interrupt() && output.goto_targets().is_empty() {
            let route_target = if has_tool_call_in_patch(&output.patch, &tool_call_key) {
                tools.clone()
            } else {
                finish.clone()
            };
            output.commands.push(Command::Goto(route_target));
        }
        Ok(output)
    })
}

fn validation_with_default_routing(
    validation: ValidationNode,
    validation_key: String,
    planner: String,
    finish: String,
) -> RuntimeNodeAction {
    let validation_action = validation.into_action();
    Arc::new(move |state, runtime_context| {
        let mut output = validation_action(state, runtime_context)?;
        if !output.has_interrupt() && output.goto_targets().is_empty() {
            let route_target = if has_validation_errors_in_patch(&output.patch, &validation_key) {
                finish.clone()
            } else {
                planner.clone()
            };
            output.commands.push(Command::Goto(route_target));
        }
        Ok(output)
    })
}

fn validation_router(
    validation_key: String,
    planner: String,
    finish: String,
) -> langgraph_core::ConditionalRouteFn {
    Arc::new(move |state, _runtime_context| {
        if has_validation_errors(state, &validation_key) {
            return Ok(vec![finish.clone()]);
        }
        Ok(vec![planner.clone()])
    })
}

fn has_tool_call(state: &State, key: &str) -> bool {
    state
        .get(key)
        .and_then(serde_json::Value::as_object)
        .and_then(|value| value.get("name"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn has_tool_call_in_patch(patch: &StatePatch, key: &str) -> bool {
    patch
        .get(key)
        .and_then(serde_json::Value::as_object)
        .and_then(|value| value.get("name"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn has_validation_errors(state: &State, key: &str) -> bool {
    state.get(key).and_then(serde_json::Value::as_array).is_some_and(|items| !items.is_empty())
}

fn has_validation_errors_in_patch(patch: &StatePatch, key: &str) -> bool {
    patch.get(key).and_then(serde_json::Value::as_array).is_some_and(|items| !items.is_empty())
}

fn noop_finish_node() -> NodeAction {
    Arc::new(|_state| Ok(BTreeMap::new()))
}
