use std::sync::Arc;

use serde_json::Value;

pub type ChannelRef = Arc<dyn Channel>;

pub trait Channel: Send + Sync {
    fn merge(&self, current: Option<&Value>, incoming: Vec<Value>) -> Value;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LastValue;

impl Channel for LastValue {
    fn merge(&self, current: Option<&Value>, incoming: Vec<Value>) -> Value {
        if let Some(last) = incoming.into_iter().last() {
            return last;
        }
        current.cloned().unwrap_or(Value::Null)
    }
}

pub type BinaryOpFn = Arc<dyn Fn(&Value, &Value) -> Value + Send + Sync>;

#[derive(Clone)]
pub struct BinaryOperatorAggregate {
    op: BinaryOpFn,
}

impl BinaryOperatorAggregate {
    #[must_use]
    pub fn new(op: BinaryOpFn) -> Self {
        Self { op }
    }
}

impl Channel for BinaryOperatorAggregate {
    fn merge(&self, current: Option<&Value>, incoming: Vec<Value>) -> Value {
        let mut iter = incoming.into_iter();
        let Some(first) = iter.next() else {
            return current.cloned().unwrap_or(Value::Null);
        };

        let seed = if let Some(existing) = current { (self.op)(existing, &first) } else { first };

        iter.fold(seed, |acc, item| (self.op)(&acc, &item))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Topic;

impl Channel for Topic {
    fn merge(&self, current: Option<&Value>, incoming: Vec<Value>) -> Value {
        let mut items = match current {
            Some(Value::Array(existing)) => existing.clone(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        };
        items.extend(incoming);
        Value::Array(items)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BinaryOperatorAggregate, Channel, LastValue, Topic};

    #[test]
    fn last_value_keeps_latest_write() {
        let channel = LastValue;
        let merged = channel.merge(Some(&json!("old")), vec![json!("a"), json!("b")]);
        assert_eq!(merged, json!("b"));
    }

    #[test]
    fn binary_operator_aggregate_reduces_all_values() {
        let channel = BinaryOperatorAggregate::new(std::sync::Arc::new(|left, right| {
            json!(left.as_i64().unwrap_or_default() + right.as_i64().unwrap_or_default())
        }));
        let merged = channel.merge(Some(&json!(3)), vec![json!(4), json!(5)]);
        assert_eq!(merged, json!(12));
    }

    #[test]
    fn topic_appends_incoming_values() {
        let channel = Topic;
        let merged = channel.merge(Some(&json!(["a"])), vec![json!("b"), json!("c")]);
        assert_eq!(merged, json!(["a", "b", "c"]));
    }
}
