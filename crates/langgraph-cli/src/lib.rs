pub mod app;
pub mod args;
pub mod error;
pub mod graph_presets;
pub mod io;

pub use error::CliError;

pub fn run_with_args<I>(args: I) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
{
    let command = args::parse_args(args)?;
    app::execute(command)
}
