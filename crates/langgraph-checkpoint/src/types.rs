use std::collections::BTreeMap;

use langgraph_core::StatePatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type ThreadId = String;
pub type CheckpointId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingWrite {
    pub node: String,
    pub patch: StatePatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub thread_id: ThreadId,
    pub checkpoint_id: CheckpointId,
    pub state: BTreeMap<String, Value>,
    pub versions_seen: BTreeMap<String, u64>,
    pub pending_writes: Vec<PendingWrite>,
}
