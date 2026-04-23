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
    #[error("ambiguous sequential route from node `{0}`")]
    AmbiguousRoute(String),
    #[error("graph execution did not reach finish point: `{0}`")]
    DidNotReachFinish(String),
    #[error("node `{node}` failed: {reason}")]
    NodeExecutionFailed { node: String, reason: String },
    #[error("channel `{field}` merge failed: {reason}")]
    ChannelMergeFailed { field: String, reason: String },
}
