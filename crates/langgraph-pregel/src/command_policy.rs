use std::collections::BTreeSet;

use langgraph_core::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicy {
    pub allow_goto: bool,
    pub allow_goto_many: bool,
    pub max_goto_targets: usize,
    pub deduplicate_targets: bool,
    pub allow_interrupt_with_routing: bool,
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self {
            allow_goto: true,
            allow_goto_many: true,
            max_goto_targets: usize::MAX,
            deduplicate_targets: true,
            allow_interrupt_with_routing: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommands {
    pub interrupted: bool,
    pub goto_targets: Vec<String>,
}

pub fn resolve_commands(
    node: &str,
    commands: &[Command],
    policy: &CommandPolicy,
) -> Result<ResolvedCommands, String> {
    let mut interrupted = false;
    let mut goto_targets = Vec::<String>::new();

    for command in commands {
        match command {
            Command::Interrupt => {
                interrupted = true;
            }
            Command::Goto(target) => {
                if !policy.allow_goto {
                    return Err(format!("command policy rejected Goto at node `{node}`"));
                }
                goto_targets.push(target.clone());
            }
            Command::GotoMany(targets) => {
                if !policy.allow_goto_many {
                    return Err(format!("command policy rejected GotoMany at node `{node}`"));
                }
                goto_targets.extend(targets.iter().cloned());
            }
        }
    }

    if goto_targets.len() > policy.max_goto_targets {
        return Err(format!(
            "command policy max_goto_targets exceeded at node `{node}`: {} > {}",
            goto_targets.len(),
            policy.max_goto_targets
        ));
    }

    if policy.deduplicate_targets {
        let mut deduplicated = BTreeSet::new();
        for target in goto_targets {
            deduplicated.insert(target);
        }
        goto_targets = deduplicated.into_iter().collect();
    }

    if interrupted && !policy.allow_interrupt_with_routing && !goto_targets.is_empty() {
        return Err(format!(
            "command policy rejected mixed interrupt and routing at node `{node}`"
        ));
    }

    Ok(ResolvedCommands { interrupted, goto_targets })
}
