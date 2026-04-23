use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::channel::ChannelRef;
use crate::error::GraphError;
use crate::state::{ReducerFn, State, StatePatch};

pub type NodeAction = Arc<dyn Fn(&State) -> Result<StatePatch, String> + Send + Sync>;

#[derive(Clone)]
pub struct StateGraph {
    nodes: BTreeMap<String, NodeAction>,
    edges: Vec<(String, String)>,
    reducers: BTreeMap<String, ReducerFn>,
    channels: BTreeMap<String, ChannelRef>,
    entry_point: Option<String>,
    finish_point: Option<String>,
}

impl StateGraph {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            reducers: BTreeMap::new(),
            channels: BTreeMap::new(),
            entry_point: None,
            finish_point: None,
        }
    }

    pub fn add_node(
        &mut self,
        name: impl Into<String>,
        action: NodeAction,
    ) -> Result<(), GraphError> {
        let name = name.into();
        if self.nodes.contains_key(&name) {
            return Err(GraphError::DuplicateNode(name));
        }
        self.nodes.insert(name, action);
        Ok(())
    }

    pub fn add_edge(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Result<(), GraphError> {
        let from = from.into();
        let to = to.into();
        self.edges.push((from, to));
        Ok(())
    }

    pub fn add_reducer(&mut self, field: impl Into<String>, reducer: ReducerFn) {
        self.reducers.insert(field.into(), reducer);
    }

    pub fn add_channel(&mut self, field: impl Into<String>, channel: ChannelRef) {
        self.channels.insert(field.into(), channel);
    }

    pub fn set_entry_point(&mut self, node: impl Into<String>) {
        self.entry_point = Some(node.into());
    }

    pub fn set_finish_point(&mut self, node: impl Into<String>) {
        self.finish_point = Some(node.into());
    }

    pub fn compile(self) -> Result<CompiledGraph, GraphError> {
        let entry = self.entry_point.ok_or(GraphError::MissingEntryPoint)?;
        let finish = self.finish_point.ok_or(GraphError::MissingFinishPoint)?;

        if !self.nodes.contains_key(&entry) {
            return Err(GraphError::NodeNotFound(entry));
        }
        if !self.nodes.contains_key(&finish) {
            return Err(GraphError::NodeNotFound(finish));
        }

        let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (from, to) in &self.edges {
            if !self.nodes.contains_key(from) || !self.nodes.contains_key(to) {
                return Err(GraphError::InvalidEdge { from: from.clone(), to: to.clone() });
            }
            adjacency
                .entry(from.clone())
                .and_modify(|nodes| nodes.push(to.clone()))
                .or_insert_with(|| vec![to.clone()]);
        }

        // 预检：finish 必须可从 entry 到达。
        let mut visited = BTreeSet::new();
        let mut stack = vec![entry.clone()];
        while let Some(node) = stack.pop() {
            if !visited.insert(node.clone()) {
                continue;
            }
            if let Some(next_nodes) = adjacency.get(&node) {
                stack.extend(next_nodes.iter().cloned());
            }
        }
        if !visited.contains(&finish) {
            return Err(GraphError::DidNotReachFinish(finish));
        }

        Ok(CompiledGraph {
            nodes: self.nodes,
            adjacency,
            reducers: self.reducers,
            channels: self.channels,
            entry_point: entry,
            finish_point: finish,
        })
    }
}

impl Default for StateGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct CompiledGraph {
    pub nodes: BTreeMap<String, NodeAction>,
    pub adjacency: BTreeMap<String, Vec<String>>,
    pub reducers: BTreeMap<String, ReducerFn>,
    pub channels: BTreeMap<String, ChannelRef>,
    pub entry_point: String,
    pub finish_point: String,
}
