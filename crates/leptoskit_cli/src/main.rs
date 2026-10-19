//! The `leptoskit` command line interface.

use clap::Parser;
use leptoskit_cli::cli::Cli;
use leptoskit_cli::cli::run;

fn main() {
	let cli = Cli::parse();
	if let Err(error) = run(cli) {
		eprintln!("error: {error:#}");
		std::process::exit(1);
	}
}
