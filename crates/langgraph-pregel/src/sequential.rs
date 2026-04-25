use std::collections::BTreeSet;

use langgraph_core::{
    apply_writes_with_versions, Command, CompiledGraph, GraphError, RuntimeContext, State,
    StatePatch, VersionMap,
};
use tokio::sync::mpsc;

use crate::command_policy::{resolve_commands, CommandPolicy};

#[derive(Debug, Default, Clone, Copy)]
pub struct SequentialExecutor;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionMetadata {
    pub supersteps: usize,
    pub channel_versions: VersionMap,
    pub versions_seen: VersionMap,
    pub command_trace: Vec<CommandTraceEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTraceEvent {
    pub node: String,
    pub superstep: usize,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionResult {
    pub state: State,
    pub metadata: ExecutionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSignal {
    pub node: String,
    pub superstep: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandExecutionResult {
    pub state: State,
    pub metadata: ExecutionMetadata,
    pub interrupt: Option<InterruptSignal>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    NodeStarted { node: String, superstep: usize },
    NodeFinished { node: String, superstep: usize, patch: StatePatch },
    StateChunk { superstep: usize, chunk: StatePatch },
    CommandEmitted { node: String, superstep: usize, command: Command },
    Interrupted { node: String, superstep: usize },
    Completed { state: State, metadata: ExecutionMetadata, interrupt: Option<InterruptSignal> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamExecutionResult {
    pub state: State,
    pub metadata: ExecutionMetadata,
    pub interrupt: Option<InterruptSignal>,
    pub events: Vec<StreamEvent>,
}

impl SequentialExecutor {
    pub fn invoke(&self, graph: &CompiledGraph, state: State) -> Result<State, GraphError> {
        let result = self.invoke_with_metadata(graph, state)?;
        Ok(result.state)
    }

    pub async fn ainvoke(&self, graph: &CompiledGraph, state: State) -> Result<State, GraphError> {
        let graph = graph.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || executor.invoke(&graph, state))
            .await
            .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_with_metadata(
        &self,
        graph: &CompiledGraph,
        state: State,
    ) -> Result<ExecutionResult, GraphError> {
        self.invoke_with_metadata_from_versions(graph, state, VersionMap::new())
    }

    pub async fn ainvoke_with_metadata(
        &self,
        graph: &CompiledGraph,
        state: State,
    ) -> Result<ExecutionResult, GraphError> {
        let graph = graph.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || executor.invoke_with_metadata(&graph, state))
            .await
            .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_with_metadata_from_versions(
        &self,
        graph: &CompiledGraph,
        state: State,
        initial_versions: VersionMap,
    ) -> Result<ExecutionResult, GraphError> {
        let command_result = self.invoke_with_runtime_context_from_versions(
            graph,
            state,
            RuntimeContext::new(),
            initial_versions,
        )?;
        if let Some(interrupt) = command_result.interrupt {
            return Err(GraphError::Interrupted { node: interrupt.node });
        }
        Ok(ExecutionResult { state: command_result.state, metadata: command_result.metadata })
    }

    pub async fn ainvoke_with_metadata_from_versions(
        &self,
        graph: &CompiledGraph,
        state: State,
        initial_versions: VersionMap,
    ) -> Result<ExecutionResult, GraphError> {
        let graph = graph.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || {
            executor.invoke_with_metadata_from_versions(&graph, state, initial_versions)
        })
        .await
        .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<CommandExecutionResult, GraphError> {
        self.invoke_with_runtime_context_and_policy(
            graph,
            state,
            runtime_context,
            VersionMap::new(),
            &CommandPolicy::default(),
        )
    }

    pub async fn ainvoke_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<CommandExecutionResult, GraphError> {
        let graph = graph.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || {
            executor.invoke_with_runtime_context(&graph, state, runtime_context)
        })
        .await
        .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_with_runtime_context_from_versions(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
    ) -> Result<CommandExecutionResult, GraphError> {
        self.invoke_with_runtime_context_and_policy(
            graph,
            state,
            runtime_context,
            initial_versions,
            &CommandPolicy::default(),
        )
    }

    pub async fn ainvoke_with_runtime_context_from_versions(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
    ) -> Result<CommandExecutionResult, GraphError> {
        let graph = graph.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || {
            executor.invoke_with_runtime_context_from_versions(
                &graph,
                state,
                runtime_context,
                initial_versions,
            )
        })
        .await
        .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub fn invoke_with_runtime_context_and_policy(
        &self,
        graph: &CompiledGraph,
        mut state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
    ) -> Result<CommandExecutionResult, GraphError> {
        let mut active = BTreeSet::from([graph.entry_point.clone()]);
        let mut step_count = 0usize;
        let mut channel_versions = initial_versions.clone();
        let mut versions_seen = initial_versions;
        let mut command_trace = Vec::<CommandTraceEvent>::new();
        const MAX_STEPS: usize = 10_000;

        while !active.is_empty() {
            step_count += 1;
            if step_count > MAX_STEPS {
                return Err(GraphError::DidNotReachFinish(graph.finish_point.clone()));
            }

            let mut writes: Vec<StatePatch> = Vec::new();
            let mut next_active = BTreeSet::new();
            let snapshot = state.clone();
            let mut interrupted_by: Option<String> = None;

            for node_name in &active {
                let node = graph
                    .nodes
                    .get(node_name)
                    .ok_or_else(|| GraphError::NodeNotFound(node_name.clone()))?;
                let output = node(&snapshot, &runtime_context).map_err(|reason| {
                    GraphError::NodeExecutionFailed { node: node_name.clone(), reason }
                })?;
                let commands = output.commands;
                let resolved =
                    resolve_commands(node_name, &commands, command_policy).map_err(|reason| {
                        GraphError::CommandRejected { node: node_name.clone(), reason }
                    })?;
                writes.push(output.patch);
                for command in &commands {
                    command_trace.push(CommandTraceEvent {
                        node: node_name.clone(),
                        superstep: step_count,
                        command: command.clone(),
                    });
                }
                if resolved.interrupted && interrupted_by.is_none() {
                    interrupted_by = Some(node_name.clone());
                }
                if interrupted_by.is_some() {
                    continue;
                }
                if !resolved.goto_targets.is_empty() {
                    for target in resolved.goto_targets {
                        if !graph.nodes.contains_key(&target) {
                            return Err(GraphError::InvalidGotoTarget {
                                node: node_name.clone(),
                                target,
                            });
                        }
                        next_active.insert(target);
                    }
                    continue;
                }
                if let Some(conditional) = graph.conditional_edges.get(node_name) {
                    let routed_targets =
                        (conditional.router)(&snapshot, &runtime_context).map_err(|reason| {
                            GraphError::NodeExecutionFailed { node: node_name.clone(), reason }
                        })?;
                    for target in routed_targets {
                        if !conditional.targets.contains(&target)
                            || !graph.nodes.contains_key(&target)
                        {
                            return Err(GraphError::InvalidConditionalTarget {
                                from: node_name.clone(),
                                to: target,
                            });
                        }
                        next_active.insert(target);
                    }
                    continue;
                }
                if let Some(next_nodes) = graph.adjacency.get(node_name) {
                    next_active.extend(next_nodes.iter().cloned());
                }
            }

            apply_writes_with_versions(
                &mut state,
                writes,
                &graph.reducers,
                &graph.channels,
                &mut channel_versions,
                &mut versions_seen,
            )?;

            if let Some(node) = interrupted_by {
                return Ok(CommandExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                        command_trace,
                    },
                    interrupt: Some(InterruptSignal { node, superstep: step_count }),
                });
            }

            if active.contains(&graph.finish_point) && next_active.is_empty() {
                return Ok(CommandExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                        command_trace,
                    },
                    interrupt: None,
                });
            }
            active = next_active;
        }

        Err(GraphError::DidNotReachFinish(graph.finish_point.clone()))
    }

    pub async fn ainvoke_with_runtime_context_and_policy(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
    ) -> Result<CommandExecutionResult, GraphError> {
        let graph = graph.clone();
        let command_policy = command_policy.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || {
            executor.invoke_with_runtime_context_and_policy(
                &graph,
                state,
                runtime_context,
                initial_versions,
                &command_policy,
            )
        })
        .await
        .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    pub async fn astream(
        &self,
        graph: &CompiledGraph,
        state: State,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, GraphError>>, GraphError> {
        self.astream_with_runtime_context(graph, state, RuntimeContext::new()).await
    }

    pub async fn astream_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, GraphError>>, GraphError> {
        self.astream_with_runtime_context_and_policy(
            graph,
            state,
            runtime_context,
            VersionMap::new(),
            &CommandPolicy::default(),
        )
        .await
    }

    pub async fn astream_with_runtime_context_and_policy(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, GraphError>>, GraphError> {
        let graph = graph.clone();
        let command_policy = command_policy.clone();
        let executor = *self;
        let (tx, rx) = mpsc::channel::<Result<StreamEvent, GraphError>>(64);
        tokio::task::spawn_blocking(move || {
            let run = executor.stream_with_runtime_context_and_policy_emit(
                &graph,
                state,
                runtime_context,
                initial_versions,
                &command_policy,
                |event| tx.blocking_send(Ok(event)).is_ok(),
            );
            if let Err(err) = run {
                let _ = tx.blocking_send(Err(err));
            }
        });
        Ok(rx)
    }

    pub async fn astream_collect(
        &self,
        graph: &CompiledGraph,
        state: State,
    ) -> Result<StreamExecutionResult, GraphError> {
        self.astream_collect_with_runtime_context(graph, state, RuntimeContext::new()).await
    }

    pub async fn astream_collect_with_runtime_context(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
    ) -> Result<StreamExecutionResult, GraphError> {
        self.astream_collect_with_runtime_context_and_policy(
            graph,
            state,
            runtime_context,
            VersionMap::new(),
            &CommandPolicy::default(),
        )
        .await
    }

    pub async fn astream_collect_with_runtime_context_and_policy(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
    ) -> Result<StreamExecutionResult, GraphError> {
        let graph = graph.clone();
        let command_policy = command_policy.clone();
        let executor = *self;
        tokio::task::spawn_blocking(move || {
            executor.stream_with_runtime_context_and_policy(
                &graph,
                state,
                runtime_context,
                initial_versions,
                &command_policy,
            )
        })
        .await
        .map_err(|err| GraphError::AsyncExecutionFailed(err.to_string()))?
    }

    fn stream_with_runtime_context_and_policy(
        &self,
        graph: &CompiledGraph,
        state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
    ) -> Result<StreamExecutionResult, GraphError> {
        let mut events = Vec::<StreamEvent>::new();
        let execution = self.stream_with_runtime_context_and_policy_emit(
            graph,
            state,
            runtime_context,
            initial_versions,
            command_policy,
            |event| {
                events.push(event);
                true
            },
        )?;
        Ok(StreamExecutionResult {
            state: execution.state,
            metadata: execution.metadata,
            interrupt: execution.interrupt,
            events,
        })
    }

    fn stream_with_runtime_context_and_policy_emit<F>(
        &self,
        graph: &CompiledGraph,
        mut state: State,
        runtime_context: RuntimeContext,
        initial_versions: VersionMap,
        command_policy: &CommandPolicy,
        mut emit: F,
    ) -> Result<CommandExecutionResult, GraphError>
    where
        F: FnMut(StreamEvent) -> bool,
    {
        let mut active = BTreeSet::from([graph.entry_point.clone()]);
        let mut step_count = 0usize;
        let mut channel_versions = initial_versions.clone();
        let mut versions_seen = initial_versions;
        let mut command_trace = Vec::<CommandTraceEvent>::new();
        const MAX_STEPS: usize = 10_000;

        while !active.is_empty() {
            step_count += 1;
            if step_count > MAX_STEPS {
                return Err(GraphError::DidNotReachFinish(graph.finish_point.clone()));
            }

            let mut writes: Vec<StatePatch> = Vec::new();
            let mut touched_keys = BTreeSet::<String>::new();
            let mut next_active = BTreeSet::new();
            let snapshot = state.clone();
            let mut interrupted_by: Option<String> = None;

            for node_name in &active {
                if !emit(StreamEvent::NodeStarted {
                    node: node_name.clone(),
                    superstep: step_count,
                }) {
                    return Ok(CommandExecutionResult {
                        state,
                        metadata: ExecutionMetadata {
                            supersteps: step_count.saturating_sub(1),
                            channel_versions,
                            versions_seen,
                            command_trace,
                        },
                        interrupt: None,
                    });
                }
                let node = graph
                    .nodes
                    .get(node_name)
                    .ok_or_else(|| GraphError::NodeNotFound(node_name.clone()))?;
                let output = node(&snapshot, &runtime_context).map_err(|reason| {
                    GraphError::NodeExecutionFailed { node: node_name.clone(), reason }
                })?;
                let patch = output.patch;
                touched_keys.extend(patch.keys().cloned());
                if !emit(StreamEvent::NodeFinished {
                    node: node_name.clone(),
                    superstep: step_count,
                    patch: patch.clone(),
                }) {
                    return Ok(CommandExecutionResult {
                        state,
                        metadata: ExecutionMetadata {
                            supersteps: step_count.saturating_sub(1),
                            channel_versions,
                            versions_seen,
                            command_trace,
                        },
                        interrupt: None,
                    });
                }
                let commands = output.commands;
                let resolved =
                    resolve_commands(node_name, &commands, command_policy).map_err(|reason| {
                        GraphError::CommandRejected { node: node_name.clone(), reason }
                    })?;
                writes.push(patch);
                for command in &commands {
                    command_trace.push(CommandTraceEvent {
                        node: node_name.clone(),
                        superstep: step_count,
                        command: command.clone(),
                    });
                    if !emit(StreamEvent::CommandEmitted {
                        node: node_name.clone(),
                        superstep: step_count,
                        command: command.clone(),
                    }) {
                        return Ok(CommandExecutionResult {
                            state,
                            metadata: ExecutionMetadata {
                                supersteps: step_count.saturating_sub(1),
                                channel_versions,
                                versions_seen,
                                command_trace,
                            },
                            interrupt: None,
                        });
                    }
                }
                if resolved.interrupted && interrupted_by.is_none() {
                    interrupted_by = Some(node_name.clone());
                }
                if interrupted_by.is_some() {
                    continue;
                }
                if !resolved.goto_targets.is_empty() {
                    for target in resolved.goto_targets {
                        if !graph.nodes.contains_key(&target) {
                            return Err(GraphError::InvalidGotoTarget {
                                node: node_name.clone(),
                                target,
                            });
                        }
                        next_active.insert(target);
                    }
                    continue;
                }
                if let Some(conditional) = graph.conditional_edges.get(node_name) {
                    let routed_targets =
                        (conditional.router)(&snapshot, &runtime_context).map_err(|reason| {
                            GraphError::NodeExecutionFailed { node: node_name.clone(), reason }
                        })?;
                    for target in routed_targets {
                        if !conditional.targets.contains(&target)
                            || !graph.nodes.contains_key(&target)
                        {
                            return Err(GraphError::InvalidConditionalTarget {
                                from: node_name.clone(),
                                to: target,
                            });
                        }
                        next_active.insert(target);
                    }
                    continue;
                }
                if let Some(next_nodes) = graph.adjacency.get(node_name) {
                    next_active.extend(next_nodes.iter().cloned());
                }
            }

            apply_writes_with_versions(
                &mut state,
                writes,
                &graph.reducers,
                &graph.channels,
                &mut channel_versions,
                &mut versions_seen,
            )?;

            let mut chunk = StatePatch::new();
            for key in touched_keys {
                if let Some(value) = state.get(&key) {
                    chunk.insert(key, value.clone());
                }
            }
            if !chunk.is_empty()
                && !emit(StreamEvent::StateChunk { superstep: step_count, chunk })
            {
                return Ok(CommandExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                        command_trace,
                    },
                    interrupt: None,
                });
            }

            if let Some(node) = interrupted_by {
                let interrupt = Some(InterruptSignal { node, superstep: step_count });
                let interrupted_node = interrupt.as_ref().map(|signal| signal.node.clone());
                if let Some(interrupted_node) = interrupted_node {
                    let _ = emit(StreamEvent::Interrupted {
                        node: interrupted_node,
                        superstep: step_count,
                    });
                }
                let result = CommandExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                        command_trace,
                    },
                    interrupt,
                };
                let _ = emit(StreamEvent::Completed {
                    state: result.state.clone(),
                    metadata: result.metadata.clone(),
                    interrupt: result.interrupt.clone(),
                });
                return Ok(result);
            }

            if active.contains(&graph.finish_point) && next_active.is_empty() {
                let result = CommandExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                        command_trace,
                    },
                    interrupt: None,
                };
                let _ = emit(StreamEvent::Completed {
                    state: result.state.clone(),
                    metadata: result.metadata.clone(),
                    interrupt: None,
                });
                return Ok(result);
            }
            active = next_active;
        }

        Err(GraphError::DidNotReachFinish(graph.finish_point.clone()))
    }
}
