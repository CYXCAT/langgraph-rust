use std::collections::BTreeMap;
use std::sync::Arc;

use langgraph_core::{
    NodeOutput, RuntimeContext, RuntimeNodeAction, State, StatePatch, StateValue,
};
use serde_json::{json, Value};

pub type ToolFn =
    Arc<dyn Fn(&StateValue, &RuntimeContext) -> Result<StateValue, String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRoutingStrategy {
    Strict,
    FallbackToDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorStrategy {
    WriteToState,
    ReturnNodeError,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    pub requested_tool: String,
    pub resolved_tool: String,
    pub input: StateValue,
}

pub type ToolAuditHook =
    Arc<dyn Fn(&ToolInvocation, &RuntimeContext) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct ToolNode {
    tools: BTreeMap<String, ToolFn>,
    aliases: BTreeMap<String, String>,
    default_tool: Option<String>,
    routing_strategy: ToolRoutingStrategy,
    error_strategy: ToolErrorStrategy,
    audit_hook: Option<ToolAuditHook>,
    request_key: String,
    result_key: String,
    error_key: String,
}

impl ToolNode {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            aliases: BTreeMap::new(),
            default_tool: None,
            routing_strategy: ToolRoutingStrategy::Strict,
            error_strategy: ToolErrorStrategy::WriteToState,
            audit_hook: None,
            request_key: String::from("tool_call"),
            result_key: String::from("tool_result"),
            error_key: String::from("tool_error"),
        }
    }

    #[must_use]
    pub fn with_request_key(mut self, request_key: impl Into<String>) -> Self {
        self.request_key = request_key.into();
        self
    }

    #[must_use]
    pub fn with_result_key(mut self, result_key: impl Into<String>) -> Self {
        self.result_key = result_key.into();
        self
    }

    #[must_use]
    pub fn with_error_key(mut self, error_key: impl Into<String>) -> Self {
        self.error_key = error_key.into();
        self
    }

    #[must_use]
    pub fn with_routing_strategy(mut self, strategy: ToolRoutingStrategy) -> Self {
        self.routing_strategy = strategy;
        self
    }

    #[must_use]
    pub fn with_error_strategy(mut self, strategy: ToolErrorStrategy) -> Self {
        self.error_strategy = strategy;
        self
    }

    #[must_use]
    pub fn with_default_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.default_tool = Some(tool_name.into());
        self
    }

    #[must_use]
    pub fn with_audit_hook(mut self, audit_hook: ToolAuditHook) -> Self {
        self.audit_hook = Some(audit_hook);
        self
    }

    pub fn add_tool(&mut self, name: impl Into<String>, tool: ToolFn) {
        self.tools.insert(name.into(), tool);
    }

    pub fn add_alias(&mut self, alias: impl Into<String>, target_tool: impl Into<String>) {
        self.aliases.insert(alias.into(), target_tool.into());
    }

    fn handle_error(&self, reason: String) -> Result<NodeOutput, String> {
        match self.error_strategy {
            ToolErrorStrategy::WriteToState => Ok(NodeOutput::from_patch(StatePatch::from([(
                self.error_key.clone(),
                json!(reason),
            )]))),
            ToolErrorStrategy::ReturnNodeError => Err(reason),
        }
    }

    fn parse_tool_request(&self, state: &State) -> Result<(String, StateValue), String> {
        let Some(tool_call) = state.get(&self.request_key) else {
            return Err(format!("missing tool request at key `{}`", self.request_key));
        };

        let Some(call_obj) = tool_call.as_object() else {
            return Err(format!("tool request `{}` must be a JSON object", self.request_key));
        };

        let tool_name =
            call_obj.get("name").and_then(serde_json::Value::as_str).unwrap_or_default();
        if tool_name.is_empty() {
            return Err(String::from("tool request is missing non-empty `name`"));
        }

        let tool_input = call_obj.get("input").cloned().unwrap_or(Value::Null);
        Ok((tool_name.to_string(), tool_input))
    }

    fn resolve_tool_name(&self, requested_tool: &str) -> Result<String, String> {
        let aliased =
            self.aliases.get(requested_tool).cloned().unwrap_or_else(|| requested_tool.to_string());
        if self.tools.contains_key(&aliased) {
            return Ok(aliased);
        }

        if self.routing_strategy == ToolRoutingStrategy::FallbackToDefault {
            if let Some(default_tool) = &self.default_tool {
                if self.tools.contains_key(default_tool) {
                    return Ok(default_tool.clone());
                }
            }
        }

        Err(format!("tool `{requested_tool}` is not registered"))
    }

    pub fn run(
        &self,
        state: &State,
        runtime_context: &RuntimeContext,
    ) -> Result<NodeOutput, String> {
        let (requested_tool, tool_input) = match self.parse_tool_request(state) {
            Ok(value) => value,
            Err(reason) => return self.handle_error(reason),
        };
        let resolved_tool = match self.resolve_tool_name(&requested_tool) {
            Ok(tool_name) => tool_name,
            Err(reason) => return self.handle_error(reason),
        };

        let invocation = ToolInvocation {
            requested_tool,
            resolved_tool: resolved_tool.clone(),
            input: tool_input.clone(),
        };
        if let Some(audit_hook) = &self.audit_hook {
            if let Err(reason) = audit_hook(&invocation, runtime_context) {
                return self.handle_error(format!("tool audit rejected invocation: {reason}"));
            }
        }

        let Some(tool) = self.tools.get(&resolved_tool) else {
            return self.handle_error(format!("tool `{resolved_tool}` is not registered"));
        };

        match tool(&tool_input, runtime_context) {
            Ok(value) => Ok(NodeOutput::from_patch(StatePatch::from([
                (self.result_key.clone(), value),
                (self.error_key.clone(), json!(null)),
            ]))),
            Err(reason) => self.handle_error(reason),
        }
    }

    #[must_use]
    pub fn into_action(self) -> RuntimeNodeAction {
        let node = Arc::new(self);
        Arc::new(move |state, runtime_context| node.run(state, runtime_context))
    }
}

impl Default for ToolNode {
    fn default() -> Self {
        Self::new()
    }
}
