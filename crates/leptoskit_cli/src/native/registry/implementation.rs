//! Which implementation runs a selected plugin on each target, and why nothing
//! does when none can. Every generator iterates these resolutions instead of
//! reading registration fields itself.

use leptoskit_config::MissingPlugins;

use super::*;

/// Registration for one platform:
/// `[package.metadata.leptoskit.runtime.<platform>]`. It takes precedence over
/// the shared Rust fields and the platform's native type.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PlatformRegistration {
	pub rust_type: Option<String>,
	pub rust_factory: Option<String>,
	#[serde(default)]
	pub rust_worker: bool,
	#[serde(default)]
	pub rust_async: bool,
	/// Explicitly opts the platform out: calls fail with `Unavailable` and the
	/// build doesn't warn.
	#[serde(default)]
	pub unavailable: bool,
}

impl PlatformRegistration {
	fn rust(&self) -> Option<RustRegistration<'_>> {
		rust_registration(
			self.rust_type.as_deref(),
			self.rust_factory.as_deref(),
			self.rust_worker,
			self.rust_async,
		)
	}

	pub(super) fn validate(&self, platform: &str) -> Result<()> {
		let rust = self.rust_type.is_some() || self.rust_factory.is_some();

		if self.unavailable {
			anyhow::ensure!(
				!rust && !self.rust_worker && !self.rust_async,
				"runtime.{platform}: unavailable = true cannot be combined with Rust registration"
			);
			return Ok(());
		}

		anyhow::ensure!(
			rust,
			"runtime.{platform} needs rust-type or rust-factory, or unavailable = true"
		);
		anyhow::ensure!(
			self.rust_type.is_none() || self.rust_factory.is_none(),
			"runtime.{platform} cannot declare both rust-type and rust-factory"
		);
		anyhow::ensure!(
			!self.rust_async || (self.rust_factory.is_some() && !self.rust_worker),
			"runtime.{platform}: rust-async requires rust-factory and cannot be combined with \
			 rust-worker"
		);

		for path in [&self.rust_type, &self.rust_factory].into_iter().flatten() {
			anyhow::ensure!(
				rust_service_path(path),
				"invalid Rust path in runtime.{platform}: use a Rust path without calls or \
				 expressions"
			);
		}

		Ok(())
	}
}

/// How the host calls a Rust plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
	/// On the UI thread, inside the guest's call.
	Sync,
	/// On a worker thread.
	Worker,
	/// On the host's async runtime.
	Async,
}

/// How the generated registrant constructs a Rust plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constructor<'a> {
	/// `Type::default()`.
	Type(&'a str),
	/// A factory function.
	Factory(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RustRegistration<'a> {
	pub constructor: Constructor<'a>,
	pub flavour: Flavour,
}

fn rust_registration<'a>(
	rust_type: Option<&'a str>,
	rust_factory: Option<&'a str>,
	worker: bool,
	asynchronous: bool,
) -> Option<RustRegistration<'a>> {
	let constructor = rust_type
		.map(Constructor::Type)
		.or(rust_factory.map(Constructor::Factory))?;
	let flavour = if asynchronous {
		Flavour::Async
	} else if worker {
		Flavour::Worker
	} else {
		Flavour::Sync
	};
	Some(RustRegistration {
		constructor,
		flavour,
	})
}

/// What runs a plugin on one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Implementation<'a> {
	Rust(RustRegistration<'a>),
	/// A Swift type, on iOS and macOS.
	Apple {
		module: &'a str,
		ty: &'a str,
	},
	/// A Kotlin type.
	Android {
		ty: &'a str,
	},
	/// An ArkTS class on HarmonyOS.
	ArkTs {
		ty: &'a str,
	},
	/// A Rust application service alone.
	Service(&'a str),
	/// A Rust application service beside a native dispatcher.
	Hybrid {
		service: &'a str,
		native: Box<Implementation<'a>>,
	},
}

impl<'a> Implementation<'a> {
	/// The Rust plugin the generated registrant installs, if any.
	pub fn rust(&self) -> Option<RustRegistration<'a>> {
		match self {
			Self::Rust(rust) => Some(*rust),
			_ => None,
		}
	}

	/// The Rust service factory the generated registrant defers, if any.
	pub fn service(&self) -> Option<&'a str> {
		match self {
			Self::Service(service) | Self::Hybrid { service, .. } => Some(service),
			_ => None,
		}
	}

	/// The native type the platform dispatcher registers, if any.
	pub fn native(&self) -> Option<&Self> {
		match self {
			Self::Apple { .. } | Self::Android { .. } | Self::ArkTs { .. } => Some(self),
			Self::Hybrid { native, .. } => Some(native),
			Self::Rust(_) | Self::Service(_) => None,
		}
	}

	fn native_kind(&self) -> Option<NativeKind> {
		match self {
			Self::Apple { .. } => Some(NativeKind::Apple),
			Self::Android { .. } => Some(NativeKind::Android),
			Self::ArkTs { .. } => Some(NativeKind::ArkTs),
			_ => None,
		}
	}
}

/// Why a selected plugin isn't registered on a target. Its guest calls fail
/// with `Unavailable`, because an unregistered plugin never resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
	/// The plugin has nothing for this platform.
	NoImplementation,
	/// The plugin has an implementation the generated host can't run yet.
	HostCannotRun(Capability),
	/// The plugin opts this platform out with `unavailable = true`.
	Unavailable,
}

/// Something a generated host may be unable to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
	Sync,
	Worker,
	Async,
	Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
	Apple,
	Android,
	ArkTs,
}

/// What each generated host can run. Widen this as hosts gain support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct HostCapabilities {
	pub sync: bool,
	pub worker: bool,
	pub asynchronous: bool,
	pub service: bool,
	pub native: Option<NativeKind>,
}

impl HostCapabilities {
	pub fn of(target: Target) -> Self {
		let everything = |native| {
			Self {
				sync: true,
				worker: true,
				asynchronous: true,
				service: true,
				native: Some(native),
			}
		};

		match target {
			Target::Ios | Target::IosSimulator | Target::Macos => everything(NativeKind::Apple),
			Target::Android | Target::AndroidX86_64 => everything(NativeKind::Android),
			// `bridge.cpp` dispatches synchronous Rust calls and ArkTS plugins
			// only.
			Target::Harmonyos | Target::HarmonyosX86_64 => {
				Self {
					sync: true,
					worker: false,
					asynchronous: false,
					service: false,
					native: Some(NativeKind::ArkTs),
				}
			}
			// The app-owned runners don't register plugins yet.
			Target::Windows | Target::Linux | Target::Tui => {
				Self {
					sync: false,
					worker: false,
					asynchronous: false,
					service: false,
					native: None,
				}
			}
		}
	}

	fn runs(self, capability: Capability) -> bool {
		match capability {
			Capability::Sync => self.sync,
			Capability::Worker => self.worker,
			Capability::Async => self.asynchronous,
			Capability::Service => self.service,
		}
	}

	fn runs_nothing(self) -> bool {
		!(self.sync || self.worker || self.asynchronous || self.service) && self.native.is_none()
	}
}

impl Plugin {
	/// What runs `self` on `target`, or why nothing does.
	///
	/// A platform table wins, then the shared Rust fields, then the platform's
	/// native type. When the host can't run the Rust flavour, the native type
	/// is used instead if there is one.
	pub fn implementation(
		&self,
		target: Target,
		host: HostCapabilities,
	) -> Result<Implementation<'_>, Missing> {
		let table = self.platform_table(target);

		if table.is_some_and(|table| table.unavailable) {
			return Err(Missing::Unavailable);
		}

		let native = self
			.native_implementation(target)
			.filter(|native| native.native_kind() == host.native);

		if let Some(rust) = table
			.and_then(PlatformRegistration::rust)
			.or_else(|| self.shared_rust())
		{
			let capability = match rust.flavour {
				Flavour::Sync => Capability::Sync,
				Flavour::Worker => Capability::Worker,
				Flavour::Async => Capability::Async,
			};
			return if host.runs(capability) {
				Ok(Implementation::Rust(rust))
			} else {
				native.ok_or(Missing::HostCannotRun(capability))
			};
		}

		if let Some(service) = self.rust_service_factory.as_deref() {
			if !host.service {
				return Err(Missing::HostCannotRun(Capability::Service));
			}
			return Ok(match native {
				Some(native) => {
					Implementation::Hybrid {
						service,
						native: Box::new(native),
					}
				}
				None => Implementation::Service(service),
			});
		}

		native.ok_or(Missing::NoImplementation)
	}

	/// Whether any registration of the plugin runs on the async host, which
	/// needs host-owned options.
	pub(super) fn declares_async(&self) -> bool {
		self.rust_async || self.platform_tables().any(|(_, table)| table.rust_async)
	}

	pub(super) fn platform_tables(
		&self,
	) -> impl Iterator<Item = (&'static str, &PlatformRegistration)> {
		[
			("ios", &self.ios),
			("macos", &self.macos),
			("android", &self.android),
			("harmonyos", &self.harmonyos),
			("windows", &self.windows),
			("linux", &self.linux),
			("tui", &self.tui),
		]
		.into_iter()
		.filter_map(|(name, table)| Some((name, table.as_ref()?)))
	}

	fn platform_table(&self, target: Target) -> Option<&PlatformRegistration> {
		match target {
			Target::Ios | Target::IosSimulator => self.ios.as_ref(),
			Target::Macos => self.macos.as_ref(),
			Target::Android | Target::AndroidX86_64 => self.android.as_ref(),
			Target::Harmonyos | Target::HarmonyosX86_64 => self.harmonyos.as_ref(),
			Target::Windows => self.windows.as_ref(),
			Target::Linux => self.linux.as_ref(),
			Target::Tui => self.tui.as_ref(),
		}
	}

	fn shared_rust(&self) -> Option<RustRegistration<'_>> {
		rust_registration(
			self.rust_type.as_deref(),
			self.rust_factory.as_deref(),
			self.rust_worker,
			self.rust_async,
		)
	}

	fn native_implementation(&self, target: Target) -> Option<Implementation<'_>> {
		match target {
			Target::Ios | Target::IosSimulator | Target::Macos => {
				Some(Implementation::Apple {
					module: self.apple_module.as_deref()?,
					ty: self.apple_type.as_deref()?,
				})
			}
			Target::Android | Target::AndroidX86_64 => {
				Some(Implementation::Android {
					ty: self.android_type.as_deref()?,
				})
			}
			Target::Harmonyos | Target::HarmonyosX86_64 => {
				Some(Implementation::ArkTs {
					ty: self.harmonyos_type.as_deref()?,
				})
			}
			Target::Windows | Target::Linux | Target::Tui => None,
		}
	}
}

/// A selected plugin left out of one target's build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPlugin {
	pub name: String,
	pub target: Target,
	pub reason: Missing,
}

impl MissingPlugin {
	/// Why the plugin isn't registered, for example "haptics has no Linux
	/// implementation".
	pub fn describe(&self) -> String {
		let name = &self.name;
		let (platform, adjective) = match self.target {
			Target::Ios | Target::IosSimulator => ("iOS", "iOS"),
			Target::Macos => ("macOS", "macOS"),
			Target::Android | Target::AndroidX86_64 => ("Android", "Android"),
			Target::Harmonyos | Target::HarmonyosX86_64 => ("HarmonyOS", "HarmonyOS"),
			Target::Windows => ("Windows", "Windows"),
			Target::Linux => ("Linux", "Linux"),
			Target::Tui => ("the terminal", "terminal"),
		};
		let host = match self.target {
			Target::Windows | Target::Linux | Target::Tui => format!("the {adjective} runner"),
			_ => format!("the {adjective} host"),
		};

		match self.reason {
			Missing::NoImplementation => format!("{name} has no {adjective} implementation"),
			Missing::Unavailable => format!("{name} is unavailable on {platform}"),
			Missing::HostCannotRun(_) if HostCapabilities::of(self.target).runs_nothing() => {
				format!("{name} can't run on {platform}: {host} doesn't run plugins yet")
			}
			Missing::HostCannotRun(capability) => {
				let what = match capability {
					Capability::Sync => "Rust plugins",
					Capability::Worker => "worker plugins",
					Capability::Async => "async plugins",
					Capability::Service => "plugin services",
				};
				format!("{name} can't run on {platform}: {host} doesn't run {what} yet")
			}
		}
	}

	/// The build warning, or `None` when the plugin opted the platform out.
	pub fn warning(&self) -> Option<String> {
		(self.reason != Missing::Unavailable)
			.then(|| format!("warning: {}; calls fail with Unavailable", self.describe()))
	}
}

/// Keep the plugins `target` can run and record the rest. With
/// `native.missing-plugins = "error"`, any plugin left out fails the build.
pub(super) fn select_for_target(
	plugins: Vec<Plugin>,
	target: Target,
	policy: MissingPlugins,
) -> Result<(Vec<Plugin>, Vec<MissingPlugin>)> {
	let host = HostCapabilities::of(target);
	let mut kept = Vec::new();
	let mut missing = Vec::new();

	for plugin in plugins {
		match plugin.implementation(target, host) {
			Ok(_) => kept.push(plugin),
			Err(reason) => {
				missing.push(MissingPlugin {
					name: plugin.name.clone(),
					target,
					reason,
				});
			}
		}
	}

	if policy == MissingPlugins::Error && !missing.is_empty() {
		let reasons: Vec<_> = missing.iter().map(MissingPlugin::describe).collect();
		anyhow::bail!(
			"native.missing-plugins = \"error\" stops the build: {}",
			reasons.join("; ")
		);
	}

	Ok((kept, missing))
}

impl Registry {
	/// The selected plugins with what runs each on `target`, in selection
	/// order. Plugins the target can't run were removed when the registry
	/// was resolved, so finding one here is an error.
	pub fn implementations(
		&self,
		target: Target,
	) -> Result<Vec<(usize, &Plugin, Implementation<'_>)>> {
		let host = HostCapabilities::of(target);
		self.plugins
			.iter()
			.enumerate()
			.map(|(index, plugin)| {
				let implementation = plugin.implementation(target, host).map_err(|reason| {
					anyhow::anyhow!(
						"{}",
						MissingPlugin {
							name: plugin.name.clone(),
							target,
							reason,
						}
						.describe()
					)
				})?;
				Ok((index, plugin, implementation))
			})
			.collect()
	}

	/// Build warnings for the selected plugins this target leaves out.
	pub fn warnings(&self) -> impl Iterator<Item = String> + '_ {
		self.missing.iter().filter_map(MissingPlugin::warning)
	}

	/// Remove the plugins `target` can't run, as resolving a registry does.
	#[cfg(test)]
	pub fn select(&mut self, target: Target, policy: MissingPlugins) -> Result<()> {
		let (plugins, missing) =
			select_for_target(std::mem::take(&mut self.plugins), target, policy)?;
		self.plugins = plugins;
		self.missing.extend(missing);
		Ok(())
	}
}
