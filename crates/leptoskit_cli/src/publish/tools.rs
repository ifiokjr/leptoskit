//! The platform tools publishing runs, such as `xcodebuild` and `xcrun`,
//! behind a small seam: each run is a [`Tool`] value, so tests check the
//! exact arguments without the tools, a signing identity, or an SDK.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;

/// One run of a tool.
#[derive(Debug, Clone)]
pub struct Tool {
	pub program: String,
	pub args: Vec<String>,
	/// The directory it runs in; the current one when `None`.
	pub dir: Option<PathBuf>,
	/// Environment variables it must not see.
	pub unset: Vec<&'static str>,
	/// Environment variables it gets, such as signing passwords, which
	/// never print.
	pub env: Vec<(&'static str, super::secrets::Secret)>,
}

impl Tool {
	pub fn new(program: &str) -> Self {
		Self {
			program: program.to_owned(),
			args: Vec::new(),
			dir: None,
			unset: Vec::new(),
			env: Vec::new(),
		}
	}

	#[must_use]
	pub fn arg(mut self, arg: impl AsRef<str>) -> Self {
		self.args.push(arg.as_ref().to_owned());
		self
	}

	#[must_use]
	pub fn args<I, S>(mut self, args: I) -> Self
	where
		I: IntoIterator<Item = S>,
		S: AsRef<str>,
	{
		self.args
			.extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
		self
	}

	#[must_use]
	pub fn path(self, path: &Path) -> Self {
		self.arg(path.display().to_string())
	}

	#[must_use]
	pub fn dir(mut self, dir: &Path) -> Self {
		self.dir = Some(dir.to_path_buf());
		self
	}

	#[must_use]
	pub fn unset(mut self, name: &'static str) -> Self {
		self.unset.push(name);
		self
	}

	#[must_use]
	pub fn env(mut self, name: &'static str, value: super::secrets::Secret) -> Self {
		self.env.push((name, value));
		self
	}

	/// The program and its first argument, for messages.
	fn name(&self) -> String {
		match self.args.first() {
			Some(first) if !first.starts_with('-') => format!("{} {first}", self.program),
			_ => self.program.clone(),
		}
	}

	fn command(&self) -> Command {
		let mut command = Command::new(&self.program);
		command.args(&self.args);
		if let Some(dir) = &self.dir {
			command.current_dir(dir);
		}
		for name in &self.unset {
			command.env_remove(name);
		}
		for (name, value) in &self.env {
			command.env(name, value.expose());
		}
		command
	}
}

/// Runs tools.
pub trait Runner {
	/// Runs `tool` to the end, with its output on the terminal.
	///
	/// # Errors
	///
	/// When it can't start, or fails.
	fn run(&self, tool: &Tool) -> Result<()>;

	/// Runs `tool` and returns what it printed.
	///
	/// # Errors
	///
	/// When it can't start, or fails.
	fn output(&self, tool: &Tool) -> Result<String>;
}

/// The machine's own tools.
pub struct System;

impl Runner for System {
	fn run(&self, tool: &Tool) -> Result<()> {
		let status = tool
			.command()
			.status()
			.with_context(|| format!("could not run {}", tool.program))?;
		anyhow::ensure!(status.success(), "{} failed: {status}", tool.name());
		Ok(())
	}

	fn output(&self, tool: &Tool) -> Result<String> {
		let output = tool
			.command()
			.output()
			.with_context(|| format!("could not run {}", tool.program))?;
		anyhow::ensure!(
			output.status.success(),
			"{} failed: {}",
			tool.name(),
			String::from_utf8_lossy(&output.stderr).trim()
		);
		Ok(String::from_utf8_lossy(&output.stdout).into_owned())
	}
}

/// Records every tool instead of running it, answering each with `answer`.
#[cfg(test)]
pub struct Recorder {
	pub seen: std::cell::RefCell<Vec<Tool>>,
	pub answer: Box<dyn Fn(&Tool) -> String>,
}

#[cfg(test)]
impl Default for Recorder {
	fn default() -> Self {
		Self {
			seen: std::cell::RefCell::default(),
			answer: Box::new(|_| String::new()),
		}
	}
}

#[cfg(test)]
impl Runner for Recorder {
	fn run(&self, tool: &Tool) -> Result<()> {
		self.seen.borrow_mut().push(tool.clone());
		Ok(())
	}

	fn output(&self, tool: &Tool) -> Result<String> {
		self.seen.borrow_mut().push(tool.clone());
		Ok((self.answer)(tool))
	}
}
