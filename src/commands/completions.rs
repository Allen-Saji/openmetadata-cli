//! `omd completions <shell>` — emit a shell completion script to stdout.

use crate::error::CliResult;
use clap::CommandFactory;
use clap_complete::{generate, Shell};

#[derive(clap::Args, Debug)]
pub struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: Shell,
}

pub fn run<C: CommandFactory>(args: CompletionsArgs) -> CliResult<()> {
    let mut cmd = C::command();
    let bin = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(())
}
