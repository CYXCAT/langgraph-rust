use std::collections::BTreeSet;

use langgraph_core::{apply_writes, CompiledGraph, GraphError, State, StatePatch};

#[derive(Debug, Default, Clone, Copy)]
pub struct SequentialExecutor;

impl SequentialExecutor {
    pub fn invoke(&self, graph: &CompiledGraph, mut state: State) -> Result<State, GraphError> {
        let mut active = BTreeSet::from([graph.entry_point.clone()]);
        let mut step_count = 0usize;
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

            apply_writes(&mut state, writes, &graph.reducers, &graph.channels);

            if active.contains(&graph.finish_point) {
                return Ok(state);
            }
            active = next_active;
        }

        Err(GraphError::DidNotReachFinish(graph.finish_point.clone()))
    }
}
