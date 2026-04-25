use std::sync::Arc;

use langgraph_core::State;

use super::{ValidationIssue, ValidationRule};

pub trait Validator: Send + Sync {
    fn validate(&self, state: &State) -> Vec<ValidationIssue>;
}

pub type ValidatorRef = Arc<dyn Validator>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositeMode {
    All,
    Any,
}

pub struct RuleSetValidator {
    rules: Vec<ValidationRule>,
}

impl RuleSetValidator {
    #[must_use]
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, rule: ValidationRule) {
        self.rules.push(rule);
    }

    #[must_use]
    pub fn into_ref(self) -> ValidatorRef {
        Arc::new(self)
    }
}

impl Default for RuleSetValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for RuleSetValidator {
    fn validate(&self, state: &State) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        for rule in &self.rules {
            if let Err(issue) = rule.validate(state.get(rule.field())) {
                issues.push(issue);
            }
        }
        issues
    }
}

pub struct CompositeValidator {
    validators: Vec<ValidatorRef>,
    mode: CompositeMode,
}

impl CompositeValidator {
    #[must_use]
    pub fn all(validators: Vec<ValidatorRef>) -> Self {
        Self { validators, mode: CompositeMode::All }
    }

    #[must_use]
    pub fn any(validators: Vec<ValidatorRef>) -> Self {
        Self { validators, mode: CompositeMode::Any }
    }

    #[must_use]
    pub fn into_ref(self) -> ValidatorRef {
        Arc::new(self)
    }
}

impl Validator for CompositeValidator {
    fn validate(&self, state: &State) -> Vec<ValidationIssue> {
        if self.validators.is_empty() {
            return Vec::new();
        }

        let mut all_issues = Vec::new();
        let mut has_success = false;
        for validator in &self.validators {
            let issues = validator.validate(state);
            if issues.is_empty() {
                has_success = true;
            }
            all_issues.extend(issues);
        }

        match self.mode {
            CompositeMode::All => all_issues,
            CompositeMode::Any => {
                if has_success {
                    Vec::new()
                } else {
                    all_issues
                }
            }
        }
    }
}
