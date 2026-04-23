use std::collections::BTreeSet;

use langgraph_core::{
    apply_writes_with_versions, CompiledGraph, GraphError, State, StatePatch, VersionMap,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct SequentialExecutor;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionMetadata {
    pub supersteps: usize,
    pub channel_versions: VersionMap,
    pub versions_seen: VersionMap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionResult {
    pub state: State,
    pub metadata: ExecutionMetadata,
}

impl SequentialExecutor {
    pub fn invoke(&self, graph: &CompiledGraph, state: State) -> Result<State, GraphError> {
        let result = self.invoke_with_metadata(graph, state)?;
        Ok(result.state)
    }

    pub fn invoke_with_metadata(
        &self,
        graph: &CompiledGraph,
        mut state: State,
    ) -> Result<ExecutionResult, GraphError> {
        let mut active = BTreeSet::from([graph.entry_point.clone()]);
        let mut step_count = 0usize;
        let mut channel_versions = VersionMap::new();
        let mut versions_seen = VersionMap::new();
        const MAX_STEPS: usize = 10_000;

        while !active.is_empty() {
            step_count += 1;
            if step_count > MAX_STEPS {
                return Err(GraphError::DidNotReachFinish(graph.finish_point.clone()));
            }

            let mut writes: Vec<StatePatch> = Vec::new();
            let mut next_active = BTreeSet::new();
            let snapshot = state.clone();

            for node_name in &active {
                let node = graph
                    .nodes
                    .get(node_name)
                    .ok_or_else(|| GraphError::NodeNotFound(node_name.clone()))?;
                let patch = node(&snapshot).map_err(|reason| GraphError::NodeExecutionFailed {
                    node: node_name.clone(),
                    reason,
                })?;
                writes.push(patch);

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
            );

            if active.contains(&graph.finish_point) {
                return Ok(ExecutionResult {
                    state,
                    metadata: ExecutionMetadata {
                        supersteps: step_count,
                        channel_versions,
                        versions_seen,
                    },
                });
            }
            active = next_active;
        }

        Err(GraphError::DidNotReachFinish(graph.finish_point.clone()))
    }
}
