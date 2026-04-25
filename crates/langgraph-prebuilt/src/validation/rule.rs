use std::sync::Arc;

use langgraph_core::StateValue;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub field: Option<String>,
    pub code: String,
    pub message: String,
}

impl ValidationIssue {
    #[must_use]
    pub fn new(
        field: Option<impl Into<String>>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self { field: field.map(Into::into), code: code.into(), message: message.into() }
    }

    #[must_use]
    pub fn to_value(&self) -> StateValue {
        json!({
            "field": self.field,
            "code": self.code,
            "message": self.message,
        })
    }
}

pub type ValidationRuleFn =
    Arc<dyn Fn(Option<&StateValue>) -> Result<(), ValidationIssue> + Send + Sync>;

#[derive(Clone)]
pub struct ValidationRule {
    field: String,
    validate: ValidationRuleFn,
}

impl ValidationRule {
    #[must_use]
    pub fn new(field: impl Into<String>, validate: ValidationRuleFn) -> Self {
        Self { field: field.into(), validate }
    }

    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    pub(crate) fn validate(&self, value: Option<&StateValue>) -> Result<(), ValidationIssue> {
        (self.validate)(value)
    }

    #[must_use]
    pub fn required(field: impl Into<String>) -> Self {
        let field_name = field.into();
        Self::new(
            field_name.clone(),
            Arc::new(move |value| {
                if value.is_none() || value == Some(&json!(null)) {
                    return Err(ValidationIssue::new(
                        Some(field_name.clone()),
                        "required",
                        format!("field `{field_name}` is required"),
                    ));
                }
                Ok(())
            }),
        )
    }

    #[must_use]
    pub fn string_min_len(field: impl Into<String>, min_len: usize) -> Self {
        let field_name = field.into();
        Self::new(
            field_name.clone(),
            Arc::new(move |value| {
                let Some(value) = value else {
                    return Err(ValidationIssue::new(
                        Some(field_name.clone()),
                        "required",
                        format!("field `{field_name}` is required"),
                    ));
                };
                let Some(text) = value.as_str() else {
                    return Err(ValidationIssue::new(
                        Some(field_name.clone()),
                        "type_error",
                        format!("field `{field_name}` must be a string"),
                    ));
                };
                if text.len() < min_len {
                    return Err(ValidationIssue::new(
                        Some(field_name.clone()),
                        "min_length",
                        format!("field `{field_name}` must be at least {min_len} characters"),
                    ));
                }
                Ok(())
            }),
        )
    }
}
