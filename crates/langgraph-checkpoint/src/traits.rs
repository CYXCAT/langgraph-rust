use thiserror::Error;

use crate::types::{Checkpoint, CheckpointId, ThreadId};

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("checkpoint not found: thread `{thread_id}`, checkpoint `{checkpoint_id}`")]
    NotFound { thread_id: ThreadId, checkpoint_id: CheckpointId },
    #[error("checkpoint conflict: `{0}`")]
    Conflict(String),
    #[error("storage error: {0}")]
    Storage(String),
}

pub trait CheckpointSaver: Send + Sync {
    fn put(&self, checkpoint: Checkpoint) -> Result<(), CheckpointError>;
    fn get(
        &self,
        thread_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Checkpoint>, CheckpointError>;
    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError>;
}
