use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::Error;
use super::Finish;
use super::MAX_ARGUMENTS;
use super::MAX_OUTPUT_BYTES;
use super::MAX_REQUEST_BYTES;
use super::Output;
use super::Request;

mod process_tree;
use process_tree::ProcessTree;

/// One positional argument slot. Extra or missing arguments are always denied.
#[derive(Debug, Clone)]
pub enum ArgumentPolicy {
	Exact(String),
	OneOf(Vec<String>),
	/// The entire UTF-8 value must match, regardless of regex anchoring.
	Pattern {
		expression: Regex,
		max_bytes: usize,
	},
}
impl ArgumentPolicy {
	fn permits(&self, value: &str) -> bool {
		match self {
			Self::Exact(expected) => value == expected,
			Self::OneOf(options) => options.iter().any(|expected| expected == value),
			Self::Pattern {
				expression,
				max_bytes,
			} => value.len() <= *max_bytes && expression.is_match(value),
		}
	}

	fn valid(&self) -> bool {
		match self {
			Self::Exact(value) => value.len() <= MAX_REQUEST_BYTES && !value.contains('\0'),
			Self::OneOf(values) => {
				!values.is_empty()
					&& values
						.iter()
						.all(|value| value.len() <= MAX_REQUEST_BYTES && !value.contains('\0'))
			}
			Self::Pattern { max_bytes, .. } => *max_bytes > 0 && *max_bytes <= MAX_REQUEST_BYTES,
		}
	}
}

/// Hard limits selected by the host before a command is exposed.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
	pub stdout_bytes: usize,
	pub stderr_bytes: usize,
	/// Includes process execution and pipe draining; must be within one hour.
	/// Container cleanup can take up to five more seconds. Failed cleanup
	/// retains the invocation's capacity slot.
	pub duration: Duration,
}
impl Default for Limits {
	fn default() -> Self {
		Self {
			stdout_bytes: 256 * 1024,
			stderr_bytes: 256 * 1024,
			duration: Duration::from_secs(30),
		}
	}
}

/// Immutable command capability, validated and canonicalized by [`Shell::new`].
#[derive(Debug, Clone)]
pub struct CommandPolicy {
	/// Existing absolute executable, never searched through PATH.
	pub executable: PathBuf,
	/// Existing absolute working directory; guests cannot change it.
	pub directory: PathBuf,
	pub arguments: Vec<ArgumentPolicy>,
	/// Exact environment. The parent environment is cleared, not inherited.
	pub environment: BTreeMap<String, String>,
	pub limits: Limits,
}
impl CommandPolicy {
	fn validate(&mut self) -> Result<(), Error> {
		if !self.executable.is_absolute()
			|| !self.directory.is_absolute()
			|| self.arguments.len() > MAX_ARGUMENTS
			|| self.arguments.iter().any(|policy| !policy.valid())
			|| self.environment.iter().any(|(name, value)| {
				name.is_empty() || name.contains(['=', '\0']) || value.contains('\0')
			})
			|| self
				.limits
				.stdout_bytes
				.saturating_add(self.limits.stderr_bytes)
				> MAX_OUTPUT_BYTES
			|| self.limits.duration.is_zero()
			|| self.limits.duration > Duration::from_secs(3600)
		{
			return Err(Error::InvalidConfiguration);
		}

		for argument in &mut self.arguments {
			if let ArgumentPolicy::Pattern { expression, .. } = argument {
				*expression = Regex::new(&format!(r"\A(?:{})\z", expression.as_str()))
					.map_err(|_| Error::InvalidConfiguration)?;
			}
		}

		self.executable = self.executable.canonicalize()?;
		self.directory = self.directory.canonicalize()?;

		if !self.executable.metadata()?.is_file() || !self.directory.metadata()?.is_dir() {
			return Err(Error::InvalidConfiguration);
		}

		#[cfg(windows)]
		if self
			.executable
			.extension()
			.and_then(|extension| extension.to_str())
			.is_some_and(|extension| {
				extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
			}) {
			// Windows executes batch scripts through cmd.exe rather than a
			// direct argv boundary.
			return Err(Error::InvalidConfiguration);
		}
		Ok(())
	}
}

/// Host-owned command scopes and a shared bound on concurrent process
/// containers.
#[derive(Debug, Clone)]
pub struct Shell {
	scopes: Arc<BTreeMap<String, CommandPolicy>>,
	capacity: Arc<Semaphore>,
}
impl Shell {
	/// Configure capabilities without starting a process. Capacity must be
	/// 1–64.
	pub fn new(
		mut scopes: BTreeMap<String, CommandPolicy>,
		max_children: usize,
	) -> Result<Self, Error> {
		if scopes.is_empty() || scopes.len() > 256 || !(1..=64).contains(&max_children) {
			return Err(Error::InvalidConfiguration);
		}

		for (name, policy) in &mut scopes {
			if !super::valid_name(name) {
				return Err(Error::InvalidConfiguration);
			}

			policy.validate()?;
		}

		Ok(Self {
			scopes: Arc::new(scopes),
			capacity: Arc::new(Semaphore::new(max_children)),
		})
	}

	/// Decode a bridge request and start its bounded asynchronous worker.
	pub fn dispatch(
		&self,
		method: leptoskit_bridge::MethodId,
		payload: &[u8],
	) -> Result<RunningChild, Error> {
		if method != super::EXECUTE {
			return Err(Error::Unavailable);
		}

		self.spawn(&Request::decode(payload)?)
	}

	/// Spawn directly with no shell interpolation. Requires a live Tokio
	/// runtime with I/O and time enabled.
	///
	/// # Panics
	///
	/// Tokio panics if the active runtime was built without its I/O or time
	/// driver. Runtime configuration is a host responsibility.
	pub fn spawn(&self, request: &Request) -> Result<RunningChild, Error> {
		request.validate()?;
		let policy = self.scopes.get(&request.command).ok_or(Error::Denied)?;

		if request.arguments.len() != policy.arguments.len()
			|| !request
				.arguments
				.iter()
				.zip(&policy.arguments)
				.all(|(value, policy)| policy.permits(value))
		{
			return Err(Error::Denied);
		}
		let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::NoRuntime)?;
		let permit = self
			.capacity
			.clone()
			.try_acquire_owned()
			.map_err(|_| Error::Busy)?;
		let deadline = tokio::time::Instant::now() + policy.limits.duration;

		let mut command = Command::new(&policy.executable);
		command
			.args(&request.arguments)
			.current_dir(&policy.directory)
			.env_clear()
			.envs(&policy.environment)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());
		let mut tree = ProcessTree::spawn(command, permit)?;
		let id = tree
			.child()
			.id()
			.ok_or_else(|| Error::Worker("spawned child has no process ID".into()))?;
		let (sender, receiver) = watch::channel(false);
		let cancellation = Cancellation(sender);
		let worker = runtime.spawn(run(tree, policy.limits, deadline, receiver));
		Ok(RunningChild {
			id,
			cancellation,
			worker: Some(worker),
		})
	}
}

/// Cloneable cancellation signal. It never exposes a raw PID kill operation.
#[derive(Debug, Clone)]
pub struct Cancellation(watch::Sender<bool>);
impl Cancellation {
	/// Whether supervision has finished attempting process-container cleanup.
	#[must_use]
	pub fn is_closed(&self) -> bool {
		self.0.is_closed()
	}

	/// Idempotently ask the worker to terminate its owned process container.
	pub fn cancel(&self) {
		self.0.send_replace(true);
	}

	/// Wait until the worker ends, including its cleanup. This completion
	/// signal does not report worker errors; use [`RunningChild::wait`] for
	/// its result. The runtime must stay alive.
	pub async fn closed(&self) {
		self.0.closed().await;
	}
}

/// A supervised child. Dropping this handle signals cancellation, even if
/// wait's future is dropped. The worker retains ownership until the process
/// container is empty. Keep the runtime alive until explicit wait/closed
/// completes during shutdown.
#[derive(Debug)]
pub struct RunningChild {
	id: u32,
	cancellation: Cancellation,
	worker: Option<JoinHandle<Result<Output, Error>>>,
}
impl RunningChild {
	pub fn id(&self) -> u32 {
		self.id
	}

	pub fn cancellation(&self) -> Cancellation {
		self.cancellation.clone()
	}

	pub fn cancel(&self) {
		self.cancellation.cancel();
	}

	/// Join terminal output. Cancellation/timeout/limits are explicit Finish
	/// values.
	pub async fn wait(mut self) -> Result<Output, Error> {
		let worker = self
			.worker
			.take()
			.ok_or_else(|| Error::Worker("child worker already joined".into()))?;
		worker
			.await
			.map_err(|error| Error::Worker(error.to_string()))?
	}

	/// Request cancellation and wait for process-container cleanup.
	pub async fn cancel_and_wait(self) -> Result<Output, Error> {
		self.cancel();
		self.wait().await
	}
}
impl Drop for RunningChild {
	fn drop(&mut self) {
		self.cancel();
	}
}

async fn run(
	mut tree: ProcessTree,
	limits: Limits,
	deadline: tokio::time::Instant,
	mut cancellation: watch::Receiver<bool>,
) -> Result<Output, Error> {
	let mut stdout = tree
		.child()
		.stdout()
		.take()
		.ok_or_else(|| Error::Worker("child stdout pipe missing".into()))?;
	let mut stderr = tree
		.child()
		.stderr()
		.take()
		.ok_or_else(|| Error::Worker("child stderr pipe missing".into()))?;
	let mut out = Vec::new();
	let mut err = Vec::new();
	let mut out_buffer = [0; 8192];
	let mut err_buffer = [0; 8192];
	let mut out_open = true;
	let mut err_open = true;
	let mut exited = false;
	let timeout = tokio::time::sleep_until(deadline);
	tokio::pin!(timeout);

	let finish = loop {
		if *cancellation.borrow() {
			break Finish::Cancelled;
		}

		if exited && !out_open && !err_open {
			break Finish::Exited;
		}

		tokio::select! {
			_ = cancellation.changed() => { break Finish::Cancelled; }
			() = &mut timeout => { break Finish::TimedOut; }
			result = tree.exited(), if !exited => {
				result?;
				// Descendants can retain pipe handles after the leader exits.
				// Terminate them before draining the remaining buffered output.
				tree.stop().await?;
				exited = true;
			}
			result = stdout.read(&mut out_buffer), if out_open => {

				let count = match result { Ok(count) => count, Err(error) => { tree.stop().await?; return Err(error.into()); } };
				out_open = count != 0;
				let remaining = limits.stdout_bytes - out.len();
				out.extend_from_slice(&out_buffer[..count.min(remaining)]);
				if count > remaining { break Finish::StdoutLimit; }
			}
			result = stderr.read(&mut err_buffer), if err_open => {
				let count = match result { Ok(count) => count, Err(error) => { tree.stop().await?; return Err(error.into()); } };
				err_open = count != 0;
				let remaining = limits.stderr_bytes - err.len();
				err.extend_from_slice(&err_buffer[..count.min(remaining)]);
				if count > remaining { break Finish::StderrLimit; }
			}
		}
	};

	// Closing pipes before reaping avoids waiting on a descendant that
	// inherited them.
	drop(stdout);
	drop(stderr);
	let status = tree.stop().await?;
	Ok(Output {
		code: status.code(),
		success: finish == Finish::Exited && status.success(),
		finish,
		stdout: out.into(),
		stderr: err.into(),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn regex_slots_are_anchored_and_enumerations_are_exact() {
		let mut policy = CommandPolicy {
			executable: std::env::current_exe().unwrap(),
			directory: std::env::current_dir().unwrap(),
			arguments: vec![ArgumentPolicy::Pattern {
				expression: Regex::new("a|ab").unwrap(),
				max_bytes: 2,
			}],
			environment: BTreeMap::new(),
			limits: Limits::default(),
		};
		policy.validate().unwrap();
		assert!(policy.arguments[0].permits("a"));
		assert!(policy.arguments[0].permits("ab"));
		assert!(!policy.arguments[0].permits("xab"));
		assert!(!policy.arguments[0].permits("a\n"));
		let enumeration = ArgumentPolicy::OneOf(vec!["status".into(), "version".into()]);
		assert!(enumeration.permits("status"));

		assert!(!enumeration.permits("status;anything"));
	}
}
