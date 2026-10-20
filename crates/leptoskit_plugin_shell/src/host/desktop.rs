use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use regex::Regex;
use serde::Deserialize;

use crate::ArgumentPolicy;
use crate::Cancellation;
use crate::CommandPolicy;
use crate::EXECUTE;
use crate::Limits;
use crate::Shell;

const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
	commands: BTreeMap<String, CommandOptions>,
	#[serde(default = "max_children")]
	max_children: usize,
}
fn max_children() -> usize {
	4
}
fn output_bytes() -> usize {
	256 * 1024
}
fn timeout_ms() -> u64 {
	30_000
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandOptions {
	executable: PathBuf,
	directory: PathBuf,
	arguments: Vec<ArgumentOptions>,
	#[serde(default)]
	environment: BTreeMap<String, String>,
	#[serde(default = "output_bytes")]
	stdout_bytes: usize,
	#[serde(default = "output_bytes")]
	stderr_bytes: usize,
	#[serde(default = "timeout_ms")]
	timeout_ms: u64,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ArgumentOptions {
	Exact {
		value: String,
	},
	OneOf {
		values: Vec<String>,
	},
	Pattern {
		expression: String,
		max_bytes: usize,
	},
}
impl TryFrom<ArgumentOptions> for ArgumentPolicy {
	type Error = BridgeError;

	fn try_from(options: ArgumentOptions) -> Result<Self, Self::Error> {
		Ok(match options {
			ArgumentOptions::Exact { value } => Self::Exact(value),
			ArgumentOptions::OneOf { values } => Self::OneOf(values),
			ArgumentOptions::Pattern {
				expression,
				max_bytes,
			} => {
				Self::Pattern {
					expression: Regex::new(&expression)
						.map_err(|_| BridgeError::InvalidArgument)?,
					max_bytes,
				}
			}
		})
	}
}

/// A generated async registration with immutable command policies. The guest
/// chooses a command name and arguments; it cannot override executable, working
/// directory, environment, deadline or capture limits.
pub struct ShellPlugin {
	shell: Shell,
	state: State,
	reply_limit: usize,
}
struct State {
	closed: CancellationToken,
	registry: Mutex<Registry>,
	max_children: usize,
}
#[derive(Default)]
struct Registry {
	next: u64,
	children: BTreeMap<u64, Cancellation>,
}

/// Parses strict host-only JSON options and validates all command policies.
/// Missing/extra fields, relative paths, unbounded argument slots, invalid
/// limits, and empty scopes are rejected before any process can start.
pub fn from_config(options_json: &str) -> Result<ShellPlugin, BridgeError> {
	if options_json.len() > MAX_CONFIG_BYTES {
		return Err(BridgeError::TooLarge);
	}
	let options: Options =
		serde_json::from_str(options_json).map_err(|_| BridgeError::InvalidArgument)?;
	let mut policies = BTreeMap::new();
	let mut reply_limit = 24;
	for (name, command) in options.commands {
		let limit = command
			.stdout_bytes
			.checked_add(command.stderr_bytes)
			.ok_or(BridgeError::TooLarge)?;
		if limit > crate::MAX_OUTPUT_BYTES {
			return Err(BridgeError::TooLarge);
		}
		reply_limit = reply_limit.max(limit + 24);
		policies.insert(
			name,
			CommandPolicy {
				executable: command.executable,
				directory: command.directory,
				arguments: command
					.arguments
					.into_iter()
					.map(ArgumentPolicy::try_from)
					.collect::<Result<_, _>>()?,
				environment: command.environment,
				limits: Limits {
					stdout_bytes: command.stdout_bytes,
					stderr_bytes: command.stderr_bytes,
					duration: Duration::from_millis(command.timeout_ms),
				},
			},
		);
	}
	let shell = Shell::new(policies, options.max_children).map_err(|error| error.bridge_error())?;
	Ok(ShellPlugin {
		shell,
		reply_limit,
		state: State {
			closed: CancellationToken::new(),
			registry: Mutex::new(Registry::default()),
			max_children: options.max_children,
		},
	})
}

impl State {
	fn cancellations(&self) -> Vec<Cancellation> {
		// Even if trusted plugin code panicked while updating the registry,
		// cleanup must retain and terminate every child it already registered.
		let registry = match self.registry.lock() {
			Ok(registry) => registry,
			Err(error) => error.into_inner(),
		};
		registry.children.values().cloned().collect()
	}
}
impl Drop for ShellPlugin {
	fn drop(&mut self) {
		self.state.closed.cancel();
		for child in self.state.cancellations() {
			child.cancel();
		}
	}
}

impl AsyncPlugin for ShellPlugin {
	fn name(&self) -> &'static str {
		"shell"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[EXECUTE]
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, BridgeError> {
		if method == EXECUTE {
			Ok(self.reply_limit)
		} else {
			Err(BridgeError::Unavailable)
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			self.reply_limit(method)?;
			let (id, child, signal) = {
				let mut registry = self
					.state
					.registry
					.lock()
					.map_err(|_| BridgeError::Native)?;
				if cancellation.is_cancelled() || self.state.closed.is_cancelled() {
					return Err(BridgeError::Cancelled);
				}
				// A dropped invocation leaves its cancellation handle here.
				// Retire it only after the supervisor has finished cleanup.
				registry.children.retain(|_, child| !child.is_closed());
				if registry.children.len() >= self.state.max_children {
					return Err(BridgeError::Busy);
				}
				let id = registry.next.checked_add(1).ok_or(BridgeError::Exhausted)?;
				let child = self
					.shell
					.dispatch(method, &payload)
					.map_err(|error| error.bridge_error())?;
				let signal = child.cancellation();
				registry.next = id;
				registry.children.insert(id, signal.clone());
				(id, child, signal)
			};
			let wait = child.wait();
			tokio::pin!(wait);
			let output = tokio::select! { biased;
				() = cancellation.cancelled() => { signal.cancel(); wait.await.map_err(|error| error.bridge_error()).and(Err(BridgeError::Cancelled)) }
				() = self.state.closed.cancelled() => { signal.cancel(); wait.await.map_err(|error| error.bridge_error()).and(Err(BridgeError::Cancelled)) }
				output = &mut wait => output.map_err(|error| error.bridge_error()),
			};
			self.state
				.registry
				.lock()
				.map_err(|_| BridgeError::Native)?
				.children
				.remove(&id);
			Ok(output?.encode()?.into())
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async move {
			self.state.closed.cancel();
			let children = self.state.cancellations();
			for child in &children {
				child.cancel();
			}
			for child in children {
				child.closed().await;
			}
		})
	}
}
