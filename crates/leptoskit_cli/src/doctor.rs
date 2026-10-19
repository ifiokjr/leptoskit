//! `leptoskit doctor`: checks the environment for missing tooling.

use std::process::Command;

use anyhow::Result;

/// One environment check.
struct Check {
	name: &'static str,
	details: String,
}

/// Runs the checks and prints a report.
pub fn run() -> Result<()> {
	let checks = vec![
		command_check("cargo", &["--version"]),
		command_check("rustc", &["--version"]),
		target_check("wasm32-unknown-unknown"),
		command_check("wasm-bindgen", &["--version"]),
		command_check("wasm-opt", &["--version"]),
		command_check("node", &["--version"]),
	];

	let mut missing = 0;
	for check in &checks {
		let mark = if check.details.contains("missing") {
			missing += 1;
			"✗"
		} else {
			"✓"
		};
		println!("{mark} {:<22} {}", check.name, check.details);
	}
	if missing > 0 {
		println!();
		println!("{missing} check(s) failed; native and web builds may not work until they pass.");
	}
	Ok(())
}

fn command_check(name: &'static str, args: &[&str]) -> Check {
	let details = match Command::new(name).args(args).output() {
		Ok(output) => {
			String::from_utf8_lossy(&output.stdout)
				.lines()
				.next()
				.unwrap_or("present")
				.to_owned()
		}
		Err(_) => "missing".to_owned(),
	};
	Check { name, details }
}

fn target_check(target: &'static str) -> Check {
	let details = match Command::new("rustup")
		.args(["target", "list", "--installed"])
		.output()
	{
		Ok(output) => {
			let installed = String::from_utf8_lossy(&output.stdout);
			if installed.lines().any(|line| line.trim() == target) {
				"installed".to_owned()
			} else {
				"missing".to_owned()
			}
		}
		Err(_) => "rustup missing".to_owned(),
	};
	Check {
		name: target,
		details,
	}
}
