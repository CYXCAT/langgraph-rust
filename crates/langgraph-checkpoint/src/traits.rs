use thiserror::Error;

use crate::types::Checkpoint;

#[derive(Debug, Error)]
pub enum CheckpointError {
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
    /// Returns checkpoints for a thread.
    ///
    /// Order is backend-defined and MUST NOT be relied on by callers.
    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError>;
}
