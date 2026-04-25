use std::path::PathBuf;

use langgraph_core::GraphError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(
        "usage: langgraph-cli run [--graph <name>] [--input <path>] [--pretty] [--metadata] [--stream]"
    )]
    Usage,
    #[error("unknown command `{0}`, expected `run`")]
    UnknownCommand(String),
    #[error("missing value for option `{0}`")]
    MissingOptionValue(String),
    #[error("unknown option `{0}`")]
    UnknownOption(String),
    #[error("failed to read input file `{path}`: {source}")]
    ReadInputFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read stdin: {0}")]
    ReadStdin(std::io::Error),
    #[error("input must be a JSON object")]
    InputMustBeJsonObject,
    #[error("failed to parse input JSON: {0}")]
    ParseInputJson(serde_json::Error),
    #[error("failed to serialize output JSON: {0}")]
    SerializeOutputJson(serde_json::Error),
    #[error("unknown graph preset `{0}`")]
    UnknownGraphPreset(String),
    #[error("graph compile failed: {0}")]
    GraphCompile(#[source] GraphError),
    #[error("graph execution failed: {0}")]
    GraphExecution(#[source] GraphError),
    #[error("failed to initialize async runtime: {0}")]
    RuntimeInit(std::io::Error),
    #[error("failed to write stream output to stdout: {0}")]
    WriteStdout(std::io::Error),
    #[error("option conflict: {0}")]
    OptionConflict(String),
}
