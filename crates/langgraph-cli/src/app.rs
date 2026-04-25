use crate::args::{Command, RunArgs};
use crate::error::CliError;
use crate::graph_presets::build_graph;
use crate::io::{
    format_output, format_output_with_metadata, format_stream_event_line, read_input_state,
};
use langgraph_core::{CompiledGraph, State};
use langgraph_pregel::SequentialExecutor;
use std::io::Write;

pub fn execute(command: Command) -> Result<String, CliError> {
    match command {
        Command::Run(args) => run(args),
    }
}

fn run(args: RunArgs) -> Result<String, CliError> {
    if args.pretty && args.stream {
        return Err(CliError::OptionConflict(String::from(
            "`--pretty` cannot be used with `--stream` (stream output is JSONL)",
        )));
    }
    let graph = build_graph(&args.graph)?;
    let input = read_input_state(args.input_path.as_deref())?;
    if args.stream {
        return run_stream(&graph, input);
    }
    if args.include_metadata {
        let execution = SequentialExecutor
            .invoke_with_metadata(&graph, input)
            .map_err(CliError::GraphExecution)?;
        return format_output_with_metadata(&execution.state, &execution.metadata, args.pretty);
    }
    let state = SequentialExecutor.invoke(&graph, input).map_err(CliError::GraphExecution)?;
    format_output(&state, args.pretty)
}

fn run_stream(graph: &CompiledGraph, input: State) -> Result<String, CliError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CliError::RuntimeInit)?;
    runtime.block_on(async move {
        let mut receiver =
            SequentialExecutor.astream(graph, input).await.map_err(CliError::GraphExecution)?;
        let mut stdout = std::io::stdout();
        while let Some(event) = receiver.recv().await {
            match event {
                Ok(event) => {
                    let line = format_stream_event_line(&event)?;
                    stdout.write_all(line.as_bytes()).map_err(CliError::WriteStdout)?;
                    stdout.write_all(b"\n").map_err(CliError::WriteStdout)?;
                    stdout.flush().map_err(CliError::WriteStdout)?;
                }
                Err(err) => return Err(CliError::GraphExecution(err)),
            }
        }
        Ok::<(), CliError>(())
    })?;
    Ok(String::new())
}
