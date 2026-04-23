use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::channel::ChannelRef;
use crate::error::GraphError;

pub type StateValue = Value;
pub type State = BTreeMap<String, StateValue>;
pub type StatePatch = BTreeMap<String, StateValue>;
pub type ReducerFn = Arc<dyn Fn(&StateValue, &StateValue) -> StateValue + Send + Sync>;
pub type VersionMap = BTreeMap<String, u64>;

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
) -> Result<(), GraphError> {
    let mut ignored_channel_versions = VersionMap::new();
    let mut ignored_versions_seen = VersionMap::new();
    apply_writes_with_versions(
        state,
        writes,
        reducers,
        channels,
        &mut ignored_channel_versions,
        &mut ignored_versions_seen,
    )
}

pub fn apply_writes_with_versions(
    state: &mut State,
    writes: Vec<StatePatch>,
    reducers: &BTreeMap<String, ReducerFn>,
    channels: &BTreeMap<String, ChannelRef>,
    channel_versions: &mut VersionMap,
    versions_seen: &mut VersionMap,
) -> Result<(), GraphError> {
    let mut grouped: BTreeMap<String, Vec<StateValue>> = BTreeMap::new();
    for patch in writes {
        for (key, value) in patch {
            grouped.entry(key).or_default().push(value);
        }
    }

    for (key, incoming_values) in grouped {
        if let Some(channel) = channels.get(&key) {
            let merged = channel
                .merge(state.get(&key), incoming_values)
                .map_err(|reason| GraphError::ChannelMergeFailed {
                    field: key.clone(),
                    reason,
                })?;
            bump_channel_version(channel_versions, versions_seen, &key);
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
    Ok(())
}

fn bump_channel_version(
    channel_versions: &mut VersionMap,
    versions_seen: &mut VersionMap,
    key: &str,
) {
    let version =
        channel_versions.entry(key.to_string()).and_modify(|current| *current += 1).or_insert(1);
    versions_seen.insert(key.to_string(), *version);
}
