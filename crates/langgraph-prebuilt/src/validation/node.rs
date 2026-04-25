use std::sync::Arc;

use langgraph_core::{NodeOutput, RuntimeNodeAction, State, StatePatch};
use serde_json::json;

use super::{RuleSetValidator, ValidationRule, Validator, ValidatorRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationConfig {
    pub errors_key: String,
    pub report_key: String,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            errors_key: String::from("validation_errors"),
            report_key: String::from("validation_report"),
        }
    }
}

pub struct ValidationNode {
    rules: RuleSetValidator,
    validators: Vec<ValidatorRef>,
    config: ValidationConfig,
}

impl ValidationNode {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: RuleSetValidator::new(),
            validators: Vec::new(),
            config: ValidationConfig::default(),
        }
    }

    #[must_use]
    pub fn with_output_key(mut self, output_key: impl Into<String>) -> Self {
        self.config.errors_key = output_key.into();
        self
    }

    #[must_use]
    pub fn with_report_key(mut self, report_key: impl Into<String>) -> Self {
        self.config.report_key = report_key.into();
        self
    }

    #[must_use]
    pub fn output_key(&self) -> &str {
        &self.config.errors_key
    }

    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.add_rule(rule);
    }

    pub fn add_validator(&mut self, validator: ValidatorRef) {
        self.validators.push(validator);
    }

    pub fn run(&self, state: &State) -> NodeOutput {
        let mut issues = self.rules.validate(state);
        for validator in &self.validators {
            issues.extend(validator.validate(state));
        }

        let messages = issues.iter().map(|issue| issue.message.clone()).collect::<Vec<_>>();
        let issue_values = issues.iter().map(|issue| issue.to_value()).collect::<Vec<_>>();
        NodeOutput::from_patch(StatePatch::from([
            (self.config.errors_key.clone(), json!(messages)),
            (
                self.config.report_key.clone(),
                json!({
                    "valid": issues.is_empty(),
                    "issues": issue_values,
                }),
            ),
        ]))
    }

    #[must_use]
    pub fn into_action(self) -> RuntimeNodeAction {
        let node = Arc::new(self);
        Arc::new(move |state, _runtime_context| Ok(node.run(state)))
    }
}

impl Default for ValidationNode {
    fn default() -> Self {
        Self::new()
    }
}
