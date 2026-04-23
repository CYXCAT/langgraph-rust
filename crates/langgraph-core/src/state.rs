use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::channel::ChannelRef;

pub type StateValue = Value;
pub type State = BTreeMap<String, StateValue>;
pub type StatePatch = BTreeMap<String, StateValue>;
pub type ReducerFn = Arc<dyn Fn(&StateValue, &StateValue) -> StateValue + Send + Sync>;

pub fn apply_patch(state: &mut State, patch: StatePatch, reducers: &BTreeMap<String, ReducerFn>) {
    for (key, incoming) in patch {
        match (state.get(&key), reducers.get(&key)) {
            (Some(existing), Some(reducer)) => {
                let merged = reducer(existing, &incoming);
                state.insert(key, merged);
            }
            _ => {
                state.insert(key, incoming);
            }
        }
    }
}

pub fn apply_writes(
    state: &mut State,
    writes: Vec<StatePatch>,
    reducers: &BTreeMap<String, ReducerFn>,
    channels: &BTreeMap<String, ChannelRef>,
) {
    let mut grouped: BTreeMap<String, Vec<StateValue>> = BTreeMap::new();
    for patch in writes {
        for (key, value) in patch {
            grouped.entry(key).or_default().push(value);
        }
    }

    for (key, incoming_values) in grouped {
        if let Some(channel) = channels.get(&key) {
            let merged = channel.merge(state.get(&key), incoming_values);
            state.insert(key, merged);
            continue;
        }

        if let Some(reducer) = reducers.get(&key) {
            let mut iter = incoming_values.into_iter();
            if let Some(first) = iter.next() {
                let mut merged = if let Some(current) = state.get(&key) {
                    reducer(current, &first)
                } else {
                    first
                };
                for value in iter {
                    merged = reducer(&merged, &value);
                }
                state.insert(key, merged);
            }
            continue;
        }

        if let Some(last) = incoming_values.into_iter().last() {
            state.insert(key, last);
        }
    }
}
