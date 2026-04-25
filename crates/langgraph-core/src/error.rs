use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("duplicate node: `{0}`")]
    DuplicateNode(String),
    #[error("node not found: `{0}`")]
    NodeNotFound(String),
    #[error("missing entry point")]
    MissingEntryPoint,
    #[error("missing finish point")]
    MissingFinishPoint,
    #[error("finish point `{0}` must not have outgoing edges")]
    FinishHasOutgoingEdges(String),
    #[error("invalid edge: `{from}` -> `{to}`")]
    InvalidEdge { from: String, to: String },
    #[error("invalid conditional edge source: `{0}`")]
    InvalidConditionalSource(String),
    #[error("duplicate conditional edge source: `{0}`")]
    DuplicateConditionalSource(String),
    #[error("invalid conditional edge target from `{from}` to `{to}`")]
    InvalidConditionalTarget { from: String, to: String },
    #[error("ambiguous sequential route from node `{0}`")]
    AmbiguousRoute(String),
    #[error("graph execution did not reach finish point: `{0}`")]
    DidNotReachFinish(String),
    #[error("node `{node}` failed: {reason}")]
    NodeExecutionFailed { node: String, reason: String },
    #[error("execution interrupted by node `{node}`")]
    Interrupted { node: String },
    #[error("command rejected at node `{node}`: {reason}")]
    CommandRejected { node: String, reason: String },
    #[error("node `{node}` emitted invalid goto target `{target}`")]
    InvalidGotoTarget { node: String, target: String },
    #[error("channel `{field}` merge failed: {reason}")]
    ChannelMergeFailed { field: String, reason: String },
    #[error("async execution failed: {0}")]
    AsyncExecutionFailed(String),
}
