use std::sync::Arc;

use langgraph_core::{
    CompiledGraph, GraphError, NodeOutput, RuntimeContext, RuntimeNodeAction, State, StatePatch,
    StateValue,
};
use serde_json::{json, Value};

use crate::{ReactAgentBuilder, ReactNodeNames, ToolNode, ValidationNode};

pub type ToolInputBuilder = Arc<dyn Fn(&State) -> StateValue + Send + Sync>;
pub type FinalAnswerBuilder = Arc<dyn Fn(&StateValue, &State) -> StateValue + Send + Sync>;
pub type ToolSelector = Arc<dyn Fn(&State) -> Option<String> + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultToolCallingKeys {
    pub question_key: String,
    pub tool_call_key: String,
    pub tool_result_key: String,
    pub tool_error_key: String,
    pub final_answer_key: String,
    pub tool_input_field: String,
}

impl Default for DefaultToolCallingKeys {
    fn default() -> Self {
        Self {
            question_key: String::from("question"),
            tool_call_key: String::from("tool_call"),
            tool_result_key: String::from("tool_result"),
            tool_error_key: String::from("tool_error"),
            final_answer_key: String::from("final_answer"),
            tool_input_field: String::from("query"),
        }
    }
}

pub struct DefaultToolCallingAgentBuilder {
    primary_tool_name: String,
    fallback_tool_name: Option<String>,
    tool_node: ToolNode,
    validator: Option<ValidationNode>,
    node_names: ReactNodeNames,
    keys: DefaultToolCallingKeys,
    tool_input_builder: ToolInputBuilder,
    final_answer_builder: FinalAnswerBuilder,
    tool_selector: Option<ToolSelector>,
    uses_default_tool_input_builder: bool,
}

impl DefaultToolCallingAgentBuilder {
    #[must_use]
    pub fn new(tool_name: impl Into<String>, tool_node: ToolNode) -> Self {
        let keys = DefaultToolCallingKeys::default();
        let question_key = keys.question_key.clone();
        Self {
            primary_tool_name: tool_name.into(),
            fallback_tool_name: None,
            tool_node,
            validator: None,
            node_names: ReactNodeNames::default(),
            keys,
            tool_input_builder: Arc::new(move |state| {
                state.get(&question_key).cloned().unwrap_or(Value::Null)
            }),
            final_answer_builder: Arc::new(|tool_result, _state| tool_result.clone()),
            tool_selector: None,
            uses_default_tool_input_builder: true,
        }
    }

    #[must_use]
    pub fn with_validator(mut self, validator: ValidationNode) -> Self {
        self.validator = Some(validator);
        self
    }

    #[must_use]
    pub fn with_node_names(mut self, names: ReactNodeNames) -> Self {
        self.node_names = names;
        self
    }

    #[must_use]
    pub fn with_question_key(mut self, key: impl Into<String>) -> Self {
        self.keys.question_key = key.into();
        if self.uses_default_tool_input_builder {
            let question_key = self.keys.question_key.clone();
            self.tool_input_builder = Arc::new(move |state| {
                state.get(&question_key).cloned().unwrap_or(Value::Null)
            });
        }
        self
    }

    #[must_use]
    pub fn with_tool_call_key(mut self, key: impl Into<String>) -> Self {
        self.keys.tool_call_key = key.into();
        self
    }

    #[must_use]
    pub fn with_tool_result_key(mut self, key: impl Into<String>) -> Self {
        self.keys.tool_result_key = key.into();
        self
    }

    #[must_use]
    pub fn with_tool_error_key(mut self, key: impl Into<String>) -> Self {
        self.keys.tool_error_key = key.into();
        self
    }

    #[must_use]
    pub fn with_final_answer_key(mut self, key: impl Into<String>) -> Self {
        self.keys.final_answer_key = key.into();
        self
    }

    #[must_use]
    pub fn with_tool_input_field(mut self, key: impl Into<String>) -> Self {
        self.keys.tool_input_field = key.into();
        self
    }

    #[must_use]
    pub fn with_tool_input_builder(mut self, builder: ToolInputBuilder) -> Self {
        self.tool_input_builder = builder;
        self.uses_default_tool_input_builder = false;
        self
    }

    #[must_use]
    pub fn with_final_answer_builder(mut self, builder: FinalAnswerBuilder) -> Self {
        self.final_answer_builder = builder;
        self
    }

    #[must_use]
    pub fn with_tool_selector(mut self, selector: ToolSelector) -> Self {
        self.tool_selector = Some(selector);
        self
    }

    #[must_use]
    pub fn with_fallback_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.fallback_tool_name = Some(tool_name.into());
        self
    }

    pub fn build(self) -> Result<CompiledGraph, GraphError> {
        let keys = self.keys.clone();
        let planner = default_planner_action(
            self.primary_tool_name,
            self.tool_selector,
            keys.clone(),
            self.tool_input_builder,
            self.final_answer_builder,
        );
        let mut tool_node = self
            .tool_node
            .with_request_key(keys.tool_call_key.clone())
            .with_result_key(keys.tool_result_key.clone())
            .with_error_key(keys.tool_error_key.clone());
        if let Some(fallback_tool_name) = self.fallback_tool_name {
            tool_node = tool_node
                .with_routing_strategy(crate::ToolRoutingStrategy::FallbackToDefault)
                .with_default_tool(fallback_tool_name);
        }

        let mut builder = ReactAgentBuilder::new(planner, tool_node)
            .with_tool_call_key(keys.tool_call_key)
            .with_node_names(self.node_names);
        if let Some(validator) = self.validator {
            builder = builder.with_validator(validator);
        }
        builder.build()
    }
}

pub fn build_default_tool_calling_agent(
    tool_name: impl Into<String>,
    tool_node: ToolNode,
) -> Result<CompiledGraph, GraphError> {
    DefaultToolCallingAgentBuilder::new(tool_name, tool_node).build()
}

fn default_planner_action(
    primary_tool_name: String,
    tool_selector: Option<ToolSelector>,
    keys: DefaultToolCallingKeys,
    tool_input_builder: ToolInputBuilder,
    final_answer_builder: FinalAnswerBuilder,
) -> RuntimeNodeAction {
    Arc::new(
        move |state: &State, _runtime_context: &RuntimeContext| -> Result<NodeOutput, String> {
            let mut patch = StatePatch::new();

            if let Some(tool_result) = non_null(state, &keys.tool_result_key) {
                patch.insert(
                    keys.final_answer_key.clone(),
                    final_answer_builder(tool_result, state),
                );
                patch.insert(keys.tool_call_key.clone(), json!(null));
                return Ok(NodeOutput::from_patch(patch));
            }

            if let Some(tool_error) = non_null(state, &keys.tool_error_key) {
                patch.insert(
                    keys.final_answer_key.clone(),
                    json!({
                        "error": tool_error.clone(),
                    }),
                );
                patch.insert(keys.tool_call_key.clone(), json!(null));
                return Ok(NodeOutput::from_patch(patch));
            }

            let input = tool_input_builder(state);
            let selected_tool = tool_selector
                .as_ref()
                .and_then(|selector| selector(state))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| primary_tool_name.clone());
            patch.insert(
                keys.tool_call_key.clone(),
                json!({
                    "name": selected_tool,
                    "input": {
                        keys.tool_input_field.clone(): input
                    }
                }),
            );
            Ok(NodeOutput::from_patch(patch))
        },
    )
}

fn non_null<'a>(state: &'a State, key: &str) -> Option<&'a StateValue> {
    state.get(key).filter(|value| !value.is_null())
}
