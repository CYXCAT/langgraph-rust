use langgraph_checkpoint::{
    Checkpoint, CheckpointError, CheckpointId, CheckpointSaver, PendingWrite, ThreadId,
};
use langgraph_core::{
    apply_writes_with_versions, CompiledGraph, GraphError, RuntimeContext, State, VersionMap,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{
    CommandPolicy, CommandTraceEvent, ExecutionMetadata, ExecutionResult, InterruptSignal,
    SequentialExecutor, StreamEvent,
};

#[derive(Debug, Error)]
pub enum CheckpointBridgeError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
    #[error("checkpoint `{checkpoint_id}` not found for thread `{thread_id}`")]
    CheckpointNotFound { thread_id: ThreadId, checkpoint_id: CheckpointId },
    #[error("thread `{0}` has no checkpoints to resume from")]
    EmptyThread(ThreadId),
    #[error(
        "thread `{thread_id}` interrupted at node `{node}` (superstep {superstep}), persisted as `{checkpoint_id}`"
    )]
    Interrupted { thread_id: ThreadId, checkpoint_id: CheckpointId, node: String, superstep: usize },
    #[error("command audit rejected in thread `{thread_id}`: {reason}")]
    CommandAuditRejected { thread_id: ThreadId, reason: String },
    #[error("stream ended without completed event in thread `{thread_id}`")]
    StreamClosedWithoutCompletion { thread_id: ThreadId },
    #[error("async execution failed: {0}")]
    AsyncExecutionFailed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadExecutionResult {
    pub execution: ExecutionResult,
    pub checkpoint_id: CheckpointId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadStreamEvent {
    Execution(StreamEvent),
    CheckpointPersisted { checkpoint_id: CheckpointId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedCheckpoint {
    pub checkpoint_id: CheckpointId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInterruptedResult {
    pub checkpoint_id: CheckpointId,
    pub interrupt: InterruptSignal,
    pub command_trace: Vec<CommandTraceEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreadInvocationOutcome {
    Completed(ThreadExecutionResult),
    Interrupted(ThreadInterruptedResult),
}

pub type CommandAuditHook =
    Arc<dyn Fn(&CommandTraceEvent) -> Result<(), String> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct CheckpointedSequentialExecutor {
    executor: SequentialExecutor,
    saver: Arc<dyn CheckpointSaver>,
    command_policy: CommandPolicy,
    command_audit_hook: Option<CommandAuditHook>,
}

impl CheckpointedSequentialExecutor {
    #[must_use]
    pub fn new(saver: Arc<dyn CheckpointSaver>) -> Self {
        Self {
            executor: SequentialExecutor,
            saver,
            command_policy: CommandPolicy::default(),
            command_audit_hook: None,
        }
    }

    #[must_use]
    pub fn with_command_policy(mut self, command_policy: CommandPolicy) -> Self {
        self.command_policy = command_policy;
        self
    }

    #[must_use]
    pub fn with_command_audit_hook(mut self, command_audit_hook: CommandAuditHook) -> Self {
        self.command_audit_hook = Some(command_audit_hook);
        self
    }

    pub fn invoke_thread(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let outcome = self.invoke_thread_with_runtime_context(
            graph,
            thread_id.clone(),
            state,
            RuntimeContext::new(),
        )?;
        match outcome {
            ThreadInvocationOutcome::Completed(result) => Ok(result),
            ThreadInvocationOutcome::Interrupted(result) => {
                Err(CheckpointBridgeError::Interrupted {
                    thread_id,
                    checkpoint_id: result.checkpoint_id,
                    node: result.interrupt.node,
                    superstep: result.interrupt.superstep,
                })
            }
        }
    }

    pub async fn ainvoke_thread(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let graph = graph.clone();
        let runner = self.clone();
        let thread_id = thread_id.into();
        tokio::task::spawn_blocking(move || runner.invoke_thread(&graph, thread_id, state))
            .await
            .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_thread_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<ThreadInvocationOutcome, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let command_result = self.executor.invoke_with_runtime_context_and_policy(
            graph,
            state,
            runtime_context,
            VersionMap::new(),
            &self.command_policy,
        )?;
        self.audit_commands(&thread_id, &command_result.metadata.command_trace)?;
        if let Some(interrupt) = command_result.interrupt {
            let metadata = command_result.metadata;
            let interrupted = self.persist_interrupted_checkpoint(
                thread_id,
                command_result.state,
                metadata.versions_seen,
                Vec::new(),
            )?;
            return Ok(ThreadInvocationOutcome::Interrupted(ThreadInterruptedResult {
                checkpoint_id: interrupted.checkpoint_id,
                interrupt,
                command_trace: metadata.command_trace,
            }));
        }

        let execution =
            ExecutionResult { state: command_result.state, metadata: command_result.metadata };
        let completed = self.persist_checkpoint(thread_id, execution)?;
        Ok(ThreadInvocationOutcome::Completed(completed))
    }

    pub async fn ainvoke_thread_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<ThreadInvocationOutcome, CheckpointBridgeError> {
        let graph = graph.clone();
        let runner = self.clone();
        let thread_id = thread_id.into();
        tokio::task::spawn_blocking(move || {
            runner.invoke_thread_with_runtime_context(&graph, thread_id, state, runtime_context)
        })
        .await
        .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn interrupt_thread(
        &self,
        thread_id: impl Into<ThreadId>,
        state: State,
        versions_seen: VersionMap,
        pending_writes: Vec<PendingWrite>,
    ) -> Result<InterruptedCheckpoint, CheckpointBridgeError> {
        self.persist_interrupted_checkpoint(thread_id.into(), state, versions_seen, pending_writes)
    }

    pub async fn ainterrupt_thread(
        &self,
        thread_id: impl Into<ThreadId>,
        state: State,
        versions_seen: VersionMap,
        pending_writes: Vec<PendingWrite>,
    ) -> Result<InterruptedCheckpoint, CheckpointBridgeError> {
        let runner = self.clone();
        let thread_id = thread_id.into();
        tokio::task::spawn_blocking(move || {
            runner.interrupt_thread(thread_id, state, versions_seen, pending_writes)
        })
        .await
        .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))?
    }

    fn persist_interrupted_checkpoint(
        &self,
        thread_id: ThreadId,
        state: State,
        versions_seen: VersionMap,
        pending_writes: Vec<PendingWrite>,
    ) -> Result<InterruptedCheckpoint, CheckpointBridgeError> {
        let checkpoint_id =
            self.persist_state_with_versions(thread_id, state, versions_seen, pending_writes)?;
        Ok(InterruptedCheckpoint { checkpoint_id })
    }

    pub async fn astream_thread(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
    ) -> Result<
        mpsc::Receiver<Result<ThreadStreamEvent, CheckpointBridgeError>>,
        CheckpointBridgeError,
    > {
        self.astream_thread_with_runtime_context(graph, thread_id, state, RuntimeContext::new())
            .await
    }

    pub async fn astream_thread_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<
        mpsc::Receiver<Result<ThreadStreamEvent, CheckpointBridgeError>>,
        CheckpointBridgeError,
    > {
        let thread_id = thread_id.into();
        let graph = graph.clone();
        let runner = self.clone();
        let (tx, rx) = mpsc::channel::<Result<ThreadStreamEvent, CheckpointBridgeError>>(64);
        tokio::spawn(async move {
            let stream_rx = match runner
                .executor
                .astream_with_runtime_context_and_policy(
                    &graph,
                    state,
                    runtime_context,
                    VersionMap::new(),
                    &runner.command_policy,
                )
                .await
            {
                Ok(stream_rx) => stream_rx,
                Err(err) => {
                    let _ = tx.send(Err(CheckpointBridgeError::Graph(err))).await;
                    return;
                }
            };
            runner.forward_stream_and_persist(thread_id, stream_rx, tx).await;
        });
        Ok(rx)
    }

    pub fn resume(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        checkpoint_id: impl Into<CheckpointId>,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = thread_id.into();
        let checkpoint_id = checkpoint_id.into();
        let checkpoint = self.saver.get(&thread_id, &checkpoint_id)?.ok_or_else(|| {
            CheckpointBridgeError::CheckpointNotFound {
                thread_id: thread_id.clone(),
                checkpoint_id: checkpoint_id.clone(),
            }
        })?;
        self.resume_from_checkpoint(graph, checkpoint)
    }

    pub async fn aresume(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
        checkpoint_id: impl Into<CheckpointId>,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let graph = graph.clone();
        let runner = self.clone();
        let thread_id = thread_id.into();
        let checkpoint_id = checkpoint_id.into();
        tokio::task::spawn_blocking(move || runner.resume(&graph, thread_id, checkpoint_id))
            .await
            .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))?
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

    pub async fn aresume_latest(
        &self,
        graph: &CompiledGraph,
        thread_id: impl Into<ThreadId>,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let graph = graph.clone();
        let runner = self.clone();
        let thread_id = thread_id.into();
        tokio::task::spawn_blocking(move || runner.resume_latest(&graph, thread_id))
            .await
            .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))?
    }

    fn resume_from_checkpoint(
        &self,
        graph: &CompiledGraph,
        checkpoint: Checkpoint,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let thread_id = checkpoint.thread_id.clone();
        let (state, versions_seen) = materialize_checkpoint_state(graph, checkpoint)?;
        let command_result = self.executor.invoke_with_runtime_context_and_policy(
            graph,
            state,
            RuntimeContext::new(),
            versions_seen,
            &self.command_policy,
        )?;
        self.audit_commands(&thread_id, &command_result.metadata.command_trace)?;
        if let Some(interrupt) = command_result.interrupt {
            let metadata = command_result.metadata;
            let interrupted = self.persist_interrupted_checkpoint(
                thread_id.clone(),
                command_result.state,
                metadata.versions_seen,
                Vec::new(),
            )?;
            return Err(CheckpointBridgeError::Interrupted {
                thread_id,
                checkpoint_id: interrupted.checkpoint_id,
                node: interrupt.node,
                superstep: interrupt.superstep,
            });
        }
        let execution =
            ExecutionResult { state: command_result.state, metadata: command_result.metadata };
        self.persist_checkpoint(thread_id, execution)
    }

    fn audit_commands(
        &self,
        thread_id: &str,
        command_trace: &[CommandTraceEvent],
    ) -> Result<(), CheckpointBridgeError> {
        if let Some(hook) = &self.command_audit_hook {
            for event in command_trace {
                hook(event).map_err(|reason| CheckpointBridgeError::CommandAuditRejected {
                    thread_id: thread_id.to_string(),
                    reason,
                })?;
            }
        }
        Ok(())
    }

    fn persist_checkpoint(
        &self,
        thread_id: ThreadId,
        execution: ExecutionResult,
    ) -> Result<ThreadExecutionResult, CheckpointBridgeError> {
        let checkpoint_id = self.persist_state_with_versions(
            thread_id.clone(),
            execution.state.clone(),
            execution.metadata.versions_seen.clone(),
            Vec::<PendingWrite>::new(),
        )?;
        Ok(ThreadExecutionResult { execution, checkpoint_id })
    }

    fn persist_stream_checkpoint(
        &self,
        thread_id: ThreadId,
        state: State,
        metadata: ExecutionMetadata,
    ) -> Result<CheckpointId, CheckpointBridgeError> {
        self.persist_state_with_versions(
            thread_id,
            state,
            metadata.versions_seen,
            Vec::<PendingWrite>::new(),
        )
    }

    fn finalize_stream(
        &self,
        thread_id: ThreadId,
        state: State,
        metadata: ExecutionMetadata,
    ) -> Result<CheckpointId, CheckpointBridgeError> {
        self.audit_commands(&thread_id, &metadata.command_trace)?;
        self.persist_stream_checkpoint(thread_id, state, metadata)
    }

    async fn forward_stream_and_persist(
        &self,
        thread_id: ThreadId,
        mut stream_rx: mpsc::Receiver<Result<StreamEvent, GraphError>>,
        tx: mpsc::Sender<Result<ThreadStreamEvent, CheckpointBridgeError>>,
    ) {
        let mut completion_state: Option<State> = None;
        let mut completion_metadata: Option<ExecutionMetadata> = None;
        while let Some(item) = stream_rx.recv().await {
            match item {
                Ok(event) => {
                    if let StreamEvent::Completed { state, metadata, .. } = &event {
                        completion_state = Some(state.clone());
                        completion_metadata = Some(metadata.clone());
                    }
                    if tx.send(Ok(ThreadStreamEvent::Execution(event))).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(CheckpointBridgeError::Graph(err))).await;
                    return;
                }
            }
        }

        let (state, metadata) = match (completion_state, completion_metadata) {
            (Some(state), Some(metadata)) => (state, metadata),
            _ => {
                let _ = tx
                    .send(Err(CheckpointBridgeError::StreamClosedWithoutCompletion { thread_id }))
                    .await;
                return;
            }
        };

        let runner = self.clone();
        let thread_for_blocking = thread_id.clone();
        let persist_result = tokio::task::spawn_blocking(move || {
            runner.finalize_stream(thread_for_blocking, state, metadata)
        })
        .await
        .map_err(|err| CheckpointBridgeError::AsyncExecutionFailed(err.to_string()))
        .and_then(|result| result);

        match persist_result {
            Ok(checkpoint_id) => {
                let _ = tx.send(Ok(ThreadStreamEvent::CheckpointPersisted { checkpoint_id })).await;
            }
            Err(err) => {
                let _ = tx.send(Err(err)).await;
            }
        }
    }

    fn persist_state_with_versions(
        &self,
        thread_id: ThreadId,
        state: State,
        versions_seen: VersionMap,
        pending_writes: Vec<PendingWrite>,
    ) -> Result<CheckpointId, CheckpointBridgeError> {
        const MAX_CONFLICT_RETRIES: usize = 8;
        for attempt in 0..=MAX_CONFLICT_RETRIES {
            let checkpoint_id = self.next_checkpoint_id(&thread_id)?;
            let checkpoint = Checkpoint {
                thread_id: thread_id.clone(),
                checkpoint_id: checkpoint_id.clone(),
                state: state.clone(),
                versions_seen: versions_seen.clone(),
                pending_writes: pending_writes.clone(),
            };
            match self.saver.put(checkpoint) {
                Ok(()) => return Ok(checkpoint_id),
                Err(CheckpointError::Conflict(_)) if attempt < MAX_CONFLICT_RETRIES => continue,
                Err(err) => return Err(CheckpointBridgeError::Checkpoint(err)),
            }
        }
        unreachable!("conflict retry loop must return success or error")
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
        let writes = checkpoint.pending_writes.into_iter().map(|pending| pending.patch).collect();
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
