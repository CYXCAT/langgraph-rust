use langgraph_checkpoint::{
    Checkpoint, CheckpointError, CheckpointId, CheckpointSaver, PendingWrite, ThreadId,
};
use langgraph_core::{apply_writes_with_versions, CompiledGraph, GraphError, State, VersionMap};
use thiserror::Error;

use crate::{ExecutionResult, SequentialExecutor};

#[derive(Debug, Error)]
pub enum CheckpointBridgeError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
    #[error("checkpoint `{checkpoint_id}` not found for thread `{thread_id}`")]
    CheckpointNotFound {
        thread_id: ThreadId,
        checkpoint_id: CheckpointId,
    },
    #[error("thread `{0}` has no checkpoints to resume from")]
    EmptyThread(ThreadId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadExecutionResult {
    pub execution: ExecutionResult,
    pub checkpoint_id: CheckpointId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedCheckpoint {
    pub checkpoint_id: CheckpointId,
}

pub struct CheckpointedSequentialExecutor<'a> {
    executor: SequentialExecutor,
    saver: &'a dyn CheckpointSaver,
}

impl<'a> CheckpointedSequentialExecutor<'a> {
    #[must_use]
    pub fn new(saver: &'a dyn CheckpointSaver) -> Self {
        Self {
            executor: SequentialExecutor,
            saver,
        }
    }

    pub fn invoke_thread(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let execution = self.executor.invoke_with_metadata(graph, state)?;
        self.persist_checkpoint(thread_id, execution)
    }

    pub fn interrupt_thread(
        &self,
        thread_id: impl Into<ThreadId>,
        state: State,
        versions_seen: VersionMap,
        pending_writes: Vec<PendingWrite>,
    ) -> Result<InterruptedCheckpoint, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let checkpoint_id = self.next_checkpoint_id(&thread_id)?;
        let checkpoint = Checkpoint {
            thread_id,
            checkpoint_id: checkpoint_id.clone(),
            state,
            versions_seen,
            pending_writes,
        };
        self.saver.put(checkpoint)?;
        Ok(InterruptedCheckpoint { checkpoint_id })
    }

    pub fn resume(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        checkpoint_id: impl Into<CheckpointId>,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let checkpoint_id = checkpoint_id.into();
        let checkpoint = self
            .saver
            .get(&thread_id, &checkpoint_id)?
            .ok_or_else(|| CheckpointBridgeError::CheckpointNotFound {
                thread_id: thread_id.clone(),
                checkpoint_id: checkpoint_id.clone(),
            })?;
        self.resume_from_checkpoint(graph, checkpoint)
    }

    pub fn resume_latest(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let checkpoints = self.saver.list(&thread_id)?;
        let checkpoint = select_latest_checkpoint(checkpoints)
            .ok_or_else(|| CheckpointBridgeError::EmptyThread(thread_id.clone()))?;
        self.resume_from_checkpoint(graph, checkpoint)
    }

    fn resume_from_checkpoint(
        &self,
        graph: &CompiledGraph,
        checkpoint: Checkpoint,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = checkpoint.thread_id.clone();
        let (state, versions_seen) = materialize_checkpoint_state(graph, checkpoint)?;
        let execution =
            self.executor
                .invoke_with_metadata_from_versions(graph, state, versions_seen)?;
        self.persist_checkpoint(thread_id, execution)
    }

    fn persist_checkpoint(
        &self,
        thread_id: ThreadId,
        execution: ExecutionResult,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let checkpoint_id = self.next_checkpoint_id(&thread_id)?;
        let checkpoint = Checkpoint {
            thread_id: thread_id.clone(),
            checkpoint_id: checkpoint_id.clone(),
            state: execution.state.clone(),
            versions_seen: execution.metadata.versions_seen.clone(),
            pending_writes: Vec::<PendingWrite>::new(),
        };
        self.saver.put(checkpoint)?;
        Ok(ThreadExecutionResult {
            execution,
            checkpoint_id,
        })
    }

    fn next_checkpoint_id(&self, thread_id: &str) -> Result<CheckpointId, CheckpointBridgeError> {
        let checkpoints = self.saver.list(thread_id)?;
        let next = checkpoints
            .iter()
            .filter_map(|checkpoint| parse_checkpoint_suffix(&checkpoint.checkpoint_id))
            .max()
            .map_or(1, |current| current + 1);
        Ok(format!("cp-{next:04}"))
    }
}

fn parse_checkpoint_suffix(id: &str) -> Option<u64> {
    id.strip_prefix("cp-").and_then(|suffix| suffix.parse::<u64>().ok())
}

fn select_latest_checkpoint(checkpoints: Vec<Checkpoint>) -> Option<Checkpoint> {
    checkpoints.into_iter().max_by(|left, right| {
        let left_parsed = parse_checkpoint_suffix(&left.checkpoint_id);
        let right_parsed = parse_checkpoint_suffix(&right.checkpoint_id);

        match (left_parsed, right_parsed) {
            // Prefer numeric checkpoint ordering when both ids are parseable.
            (Some(left_num), Some(right_num)) => left_num.cmp(&right_num),
            // Parseable ids win over unparsable ids.
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            // Deterministic fallback for malformed ids.
            (None, None) => left.checkpoint_id.cmp(&right.checkpoint_id),
        }
    })
}

fn materialize_checkpoint_state(
    graph: &CompiledGraph,
    checkpoint: Checkpoint,
) -> Result<(State, VersionMap), CheckpointBridgeError> {
    let mut state = checkpoint.state;
    let mut channel_versions = checkpoint.versions_seen.clone();
    let mut versions_seen = checkpoint.versions_seen;
    if !checkpoint.pending_writes.is_empty() {
        let writes = checkpoint
            .pending_writes
            .into_iter()
            .map(|pending| pending.patch)
            .collect();
        apply_writes_with_versions(
            &mut state,
            writes,
            &graph.reducers,
            &graph.channels,
            &mut channel_versions,
            &mut versions_seen,
        )?;
    }
    Ok((state, versions_seen))
}
