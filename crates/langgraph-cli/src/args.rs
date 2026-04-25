use std::path::PathBuf;

use crate::error::CliError;

const RUN_COMMAND: &str = "run";
const DEFAULT_GRAPH: &str = "append_ab";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    pub graph: String,
    pub input_path: Option<PathBuf>,
    pub pretty: bool,
    pub include_metadata: bool,
    pub stream: bool,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            graph: String::from(DEFAULT_GRAPH),
            input_path: None,
            pretty: false,
            include_metadata: false,
            stream: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(RunArgs),
}

pub fn parse_args<I>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut it = args.into_iter();
    let _binary = it.next();
    let Some(command) = it.next() else {
        return Err(CliError::Usage);
    };
    if command != RUN_COMMAND {
        return Err(CliError::UnknownCommand(command));
    }

    let mut run = RunArgs::default();
    let mut remaining = it.peekable();
    while let Some(token) = remaining.next() {
        match token.as_str() {
            "--graph" => {
                let value = remaining
                    .next()
                    .ok_or_else(|| CliError::MissingOptionValue(String::from("--graph")))?;
                run.graph = value;
            }
            "--input" => {
                let value = remaining
                    .next()
                    .ok_or_else(|| CliError::MissingOptionValue(String::from("--input")))?;
                run.input_path = Some(PathBuf::from(value));
            }
            "--pretty" => {
                run.pretty = true;
            }
            "--metadata" => {
                run.include_metadata = true;
            }
            "--stream" => {
                run.stream = true;
            }
            unknown if unknown.starts_with("--") => {
                return Err(CliError::UnknownOption(unknown.to_string()));
            }
            _ => {
                return Err(CliError::Usage);
            }
        }
    }

    Ok(Command::Run(run))
}
