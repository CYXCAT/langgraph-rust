use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use crate::traits::{CheckpointError, CheckpointSaver};
use crate::types::{Checkpoint, CheckpointId, ThreadId};

#[derive(Debug, Default)]
pub struct InMemorySaver {
    inner: RwLock<HashMap<ThreadId, BTreeMap<CheckpointId, Checkpoint>>>,
}

impl InMemorySaver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CheckpointSaver for InMemorySaver {
    fn put(&self, checkpoint: Checkpoint) -> Result<(), CheckpointError> {
        let mut guard = self.inner.write().map_err(|e| CheckpointError::Storage(e.to_string()))?;
        let entries = guard.entry(checkpoint.thread_id.clone()).or_insert_with(BTreeMap::new);
        if entries.contains_key(&checkpoint.checkpoint_id) {
            return Err(CheckpointError::Conflict(format!(
                "thread `{}` already has checkpoint `{}`",
                checkpoint.thread_id, checkpoint.checkpoint_id
            )));
        }
        entries.insert(checkpoint.checkpoint_id.clone(), checkpoint);
        Ok(())
    }

    fn get(
        &self,
        thread_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Checkpoint>, CheckpointError> {
        let guard = self.inner.read().map_err(|e| CheckpointError::Storage(e.to_string()))?;
        Ok(guard.get(thread_id).and_then(|checkpoints| checkpoints.get(checkpoint_id).cloned()))
    }

    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError> {
        let guard = self.inner.read().map_err(|e| CheckpointError::Storage(e.to_string()))?;
        let result = guard
            .get(thread_id)
            .map(|checkpoints| checkpoints.values().cloned().collect())
            .unwrap_or_default();
        Ok(result)
    }
}
