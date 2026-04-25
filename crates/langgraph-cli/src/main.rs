fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), langgraph_cli::CliError> {
    let output = langgraph_cli::run_with_args(std::env::args())?;
    if output.is_empty() {
        return Ok(());
    }
    println!("{output}");
    Ok(())
}
