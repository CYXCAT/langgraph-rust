use std::collections::BTreeMap;

use crate::state::{State, StatePatch, StateValue};

pub type RuntimeContext = BTreeMap<String, StateValue>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Interrupt,
    Goto(String),
    GotoMany(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeOutput {
    pub patch: StatePatch,
    pub commands: Vec<Command>,
}

impl NodeOutput {
    #[must_use]
    pub fn from_patch(patch: StatePatch) -> Self {
        Self { patch, commands: Vec::new() }
    }

    #[must_use]
    pub fn interrupt(patch: StatePatch) -> Self {
        Self { patch, commands: vec![Command::Interrupt] }
    }

    #[must_use]
    pub fn goto(patch: StatePatch, node: impl Into<String>) -> Self {
        Self { patch, commands: vec![Command::Goto(node.into())] }
    }

    #[must_use]
    pub fn goto_many(
        patch: StatePatch,
        nodes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            patch,
            commands: vec![Command::GotoMany(nodes.into_iter().map(Into::into).collect())],
        }
    }

    #[must_use]
    pub fn with_commands(mut self, commands: Vec<Command>) -> Self {
        self.commands.extend(commands);
        self
    }

    #[must_use]
    pub fn has_interrupt(&self) -> bool {
        self.commands.iter().any(|command| matches!(command, Command::Interrupt))
    }

    #[must_use]
    pub fn goto_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        for command in &self.commands {
            match command {
                Command::Goto(target) => targets.push(target.clone()),
                Command::GotoMany(items) => targets.extend(items.iter().cloned()),
                Command::Interrupt => {}
            }
        }
        targets
    }
}

pub type RuntimeNodeAction =
    std::sync::Arc<dyn Fn(&State, &RuntimeContext) -> Result<NodeOutput, String> + Send + Sync>;
