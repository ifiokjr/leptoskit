//! The file router as a leptoskit plugin.
//!
//! Apps depend on [`leptoskit_router`] for the runtime — the `Route` derive,
//! `Routable`, `Router`, and the web integration — and this crate is the
//! tooling half: it implements the plugin contract over the same generator,
//! writing the typed `Route` module before cargo runs and answering
//! `plugin:router routes`. It re-exports both router crates, so tooling has
//! one import point:
//!
//! ```
//! use leptoskit_plugin_router::router::Route;
//! use leptoskit_plugin_router::router::RouteParams;
//!
//! /// `src/routes/notes/[note].rs`, matching `/notes/:note`.
//! #[derive(Route, Clone, Debug, PartialEq)]
//! pub struct Note {
//! 	pub note: u64,
//! }
//!
//! assert_eq!(Note::FIELDS, ["note"]);
//! ```
//!
//! Activate it with a plugin declaration in `leptoskit.toml`:
//!
//! ```toml
//! [[plugins]]
//! name = "router"
//!
//! [plugins.options]
//! # Optional; the defaults suit `src/routes/` -> `src/routes.rs`.
//! dir = "src/routes"
//! output = "src/routes.rs"
//! ```
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use leptoskit_plugin::LeptoskitPlugin;
use leptoskit_plugin::PluginCommand;
use leptoskit_plugin::PluginCommandContext;
use leptoskit_plugin::PluginError;
use leptoskit_plugin::PluginFlag;
use leptoskit_plugin::PluginGenerateContext;
use leptoskit_plugin::PluginMetadata;
pub use leptoskit_router as router;
pub use leptoskit_router_codegen as codegen;
use leptoskit_router_codegen::ExportStyle;
use leptoskit_router_codegen::Generated;
use leptoskit_router_codegen::RouteKind;
use serde::Deserialize;

/// The plugin's configured routes directory and output module.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
	/// The routes directory, relative to the project root.
	dir: PathBuf,
	/// The generated module's path, relative to the project root. Derived
	/// from `dir` when unset.
	output: Option<PathBuf>,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			dir: PathBuf::from("src/routes"),
			output: None,
		}
	}
}

impl Settings {
	/// The generated module's path for the routes directory.
	fn output(&self) -> PathBuf {
		self.output
			.clone()
			.unwrap_or_else(|| self.dir.with_extension("rs"))
	}
}

/// The options table `leptoskit.toml` hands the plugin.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
	/// The routes directory, relative to the project root.
	dir: Option<PathBuf>,
	/// The generated module's path, relative to the project root.
	output: Option<PathBuf>,
}

/// The file router plugin: generates the typed `Route` module and answers
/// `plugin:router routes`.
#[derive(Debug, Default)]
pub struct RouterPlugin {
	settings: Mutex<Settings>,
}

impl RouterPlugin {
	/// Parses and validates an options table, returning the plugin's settings.
	fn parse(options: serde_json::Value) -> Result<Settings, PluginError> {
		let parsed: Options = serde_json::from_value(options).map_err(|error| {
			PluginError::Configure {
				plugin: "router".to_owned(),
				message: error.to_string(),
			}
		})?;
		let settings = Settings {
			dir: parsed.dir.unwrap_or_else(|| Settings::default().dir),
			output: parsed.output,
		};
		validate_directory(&settings.dir)?;
		if let Some(output) = &settings.output {
			validate_output(output)?;
		}
		Ok(settings)
	}

	fn settings(&self) -> Settings {
		self.settings
			.lock()
			.map(|settings| settings.clone())
			.unwrap_or_default()
	}
}

/// Rejects routes directories outside the project or with no module-friendly
/// name — the same rules the configuration applied to `[router] dir`.
fn validate_directory(dir: &Path) -> Result<(), PluginError> {
	let inside_project = dir.is_relative()
		&& dir
			.components()
			.all(|component| matches!(component, std::path::Component::Normal(_)));
	let module_name = dir
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| {
			let mut characters = name.chars();
			characters
				.next()
				.is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
				&& characters.all(|char| char == '_' || char.is_ascii_alphanumeric())
		});
	if inside_project && module_name {
		Ok(())
	} else {
		Err(PluginError::Configure {
			plugin: "router".to_owned(),
			message: format!(
				"dir `{}` must stay inside the project, written without `.` or `..`, with a \
				 module-friendly final segment",
				dir.display()
			),
		})
	}
}

/// Rejects output modules outside the project.
fn validate_output(output: &Path) -> Result<(), PluginError> {
	let inside_project = output.is_relative()
		&& output
			.components()
			.all(|component| matches!(component, std::path::Component::Normal(_)));
	if inside_project {
		Ok(())
	} else {
		Err(PluginError::Configure {
			plugin: "router".to_owned(),
			message: format!(
				"output `{}` must stay inside the project, written without `.` or `..`",
				output.display()
			),
		})
	}
}

/// Generates the module for `settings`, mapping route diagnostics to plugin
/// errors that name their file and line.
fn generate(root: &Path, settings: &Settings) -> Result<Generated, PluginError> {
	leptoskit_router_codegen::generate(&root.join(&settings.dir)).map_err(|diagnostics| {
		let lines = diagnostics
			.0
			.iter()
			.map(|diagnostic| format!("  {}", diagnostic.relative_to(root)))
			.collect::<Vec<_>>();
		PluginError::Hook {
			plugin: "router".to_owned(),
			message: format!(
				"{} cannot become routes:\n{}",
				settings.dir.display(),
				lines.join("\n")
			),
		}
	})
}

/// Creates the output module's parent directory, so a custom `output` option
/// can place the module somewhere the routes directory does not sit beside.
fn ensure_parent(root: &Path, output: &Path) -> Result<(), PluginError> {
	let parent = root
		.join(output)
		.parent()
		.map(Path::to_path_buf)
		.filter(|parent| !parent.as_os_str().is_empty());
	if let Some(parent) = parent {
		std::fs::create_dir_all(&parent).map_err(|error| {
			PluginError::Hook {
				plugin: "router".to_owned(),
				message: format!("could not create {}: {error}", parent.display()),
			}
		})?;
	}
	Ok(())
}

/// Prints the route table `plugin:router routes` lists after writing.
fn print_table(generated: &Generated) {
	let rows: Vec<[String; 3]> = generated
		.routes
		.iter()
		.map(|route| {
			let mut name = route.name.clone();
			if route.kind == RouteKind::NotFound {
				name.push_str(" (not found)");
			}
			if route.page == ExportStyle::Component {
				name.push_str(" (component)");
			}
			[route.pattern.clone(), name, route.file.clone()]
		})
		.collect();
	let widths = [0, 1].map(|column| {
		rows.iter()
			.map(|row| row[column].chars().count())
			.max()
			.unwrap_or(0)
	});
	for [pattern, name, file] in rows {
		println!(
			"  {pattern:<pattern_width$}  {name:<name_width$}  {file}",
			pattern_width = widths[0],
			name_width = widths[1],
		);
	}
}

impl LeptoskitPlugin for RouterPlugin {
	fn name(&self) -> &'static str {
		"router"
	}

	fn configure(&self, options: serde_json::Value) -> Result<(), PluginError> {
		let settings = Self::parse(options)?;
		self.settings
			.lock()
			.map_err(|_| {
				PluginError::Hook {
					plugin: "router".to_owned(),
					message: "the plugin's settings are locked".to_owned(),
				}
			})?
			.clone_from(&settings);
		Ok(())
	}

	fn metadata(&self) -> PluginMetadata {
		PluginMetadata {
			cli_namespace: Some("router".to_owned()),
			platforms: Vec::new(),
		}
	}

	fn commands(&self) -> Vec<PluginCommand> {
		vec![PluginCommand {
			name: "routes".to_owned(),
			about: "Writes the typed Route module from the routes directory".to_owned(),
			flags: vec![PluginFlag {
				name: "check".to_owned(),
				help: "Fail instead of writing when the committed module is out of date".to_owned(),
			}],
		}]
	}

	fn run_command(
		&self,
		context: &PluginCommandContext,
		command: &str,
		args: &[String],
	) -> Result<i32, PluginError> {
		debug_assert_eq!(command, "routes", "the registry validates the command");
		let check = args.iter().try_fold(false, |_state, argument| {
			match argument.as_str() {
				"--check" => Ok(true),
				other => {
					Err(PluginError::Hook {
						plugin: "router".to_owned(),
						message: format!("unknown argument `{other}`; routes takes only --check"),
					})
				}
			}
		})?;
		let settings = self.settings();
		let generated = generate(&context.root, &settings)?;
		let output = settings.output();
		let shown = output.display();
		if check {
			let status = leptoskit_router_codegen::status(&context.root.join(&output), &generated)
				.map_err(|error| {
					PluginError::Hook {
						plugin: "router".to_owned(),
						message: error.to_string(),
					}
				})?;
			if status != leptoskit_router_codegen::Status::Current {
				return Err(PluginError::Hook {
					plugin: "router".to_owned(),
					message: format!(
						"{shown} is out of date with {}; run `leptoskit plugin:router routes` and \
						 commit the result",
						settings.dir.display()
					),
				});
			}
			println!("{shown} is up to date");
		} else {
			ensure_parent(&context.root, &output)?;
			if leptoskit_router_codegen::write(&context.root.join(&output), &generated).map_err(
				|error| {
					PluginError::Hook {
						plugin: "router".to_owned(),
						message: error.to_string(),
					}
				},
			)? {
				println!("wrote {shown}");
			} else {
				println!("{shown} is up to date");
			}
		}
		print_table(&generated);
		Ok(0)
	}

	fn generate(&self, context: &mut PluginGenerateContext) -> Result<(), PluginError> {
		let settings = self.settings();
		let generated = generate(&context.root, &settings)?;
		let output = settings.output();
		ensure_parent(&context.root, &output)?;
		if leptoskit_router_codegen::write(&context.root.join(&output), &generated).map_err(
			|error| {
				PluginError::Hook {
					plugin: "router".to_owned(),
					message: error.to_string(),
				}
			},
		)? {
			println!(
				"wrote {} ({} routes)",
				output.display(),
				generated.routes.len()
			);
		}
		context.generated(context.root.join(&output));
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn temp_root(tag: &str) -> PathBuf {
		let root = std::env::temp_dir().join(format!("leptoskit-router-plugin-{tag}"));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("src/routes/notes")).unwrap();
		std::fs::write(root.join("src/routes/index.rs"), "pub fn page() {}\n").unwrap();
		std::fs::write(
			root.join("src/routes/notes/[note].rs"),
			"#[derive(Route)]\npub struct Note {\n\tpub note: u64,\n}\n\n#[component]\npub fn \
			 Page() {}\n",
		)
		.unwrap();
		root
	}

	#[test]
	fn defaults_and_options_shape_the_output_path() {
		let plugin = RouterPlugin::default();
		let settings = plugin.settings();
		assert_eq!(settings.dir, PathBuf::from("src/routes"));
		assert_eq!(settings.output(), PathBuf::from("src/routes.rs"));

		plugin
			.configure(json!({ "dir": "app/screens", "output": "app/screens.rs" }))
			.expect("valid options configure");
		assert_eq!(plugin.settings().output(), PathBuf::from("app/screens.rs"));

		let error = plugin
			.configure(json!({ "dir": "../outside" }))
			.expect_err("directories must stay inside the project");
		assert!(error.to_string().contains("outside"), "{error}");
		let error = plugin
			.configure(json!({ "unknown": true }))
			.expect_err("unknown options are rejected");
		assert!(error.to_string().contains("unknown"), "{error}");
	}

	#[test]
	fn generate_records_the_module_and_writes_only_on_change() {
		let root = temp_root("generate");
		let plugin = RouterPlugin::default();
		let mut context = PluginGenerateContext::new(&root, leptoskit_config::Platform::Web, false);
		plugin.generate(&mut context).expect("the module generates");
		assert_eq!(context.outputs(), [root.join("src/routes.rs")]);

		let written = std::fs::read_to_string(root.join("src/routes.rs")).unwrap();
		assert!(
			written.contains("@generated by `leptoskit routes`"),
			"{written}"
		);

		let before = std::fs::metadata(root.join("src/routes.rs"))
			.unwrap()
			.modified()
			.unwrap();
		let mut again = PluginGenerateContext::new(&root, leptoskit_config::Platform::Web, false);
		plugin
			.generate(&mut again)
			.expect("an unchanged tree is fine");
		let after = std::fs::metadata(root.join("src/routes.rs"))
			.unwrap()
			.modified()
			.unwrap();
		assert_eq!(before, after, "an unchanged tree rewrites nothing");
	}

	#[test]
	fn the_routes_command_writes_and_checks() {
		let root = temp_root("command");
		let plugin = RouterPlugin::default();
		plugin.configure(json!({})).unwrap();
		let config = leptoskit_config::LeptoskitConfig::default();
		let context = PluginCommandContext {
			root: root.clone(),
			config,
		};

		let error = plugin
			.run_command(&context, "routes", &["--check".to_owned()])
			.expect_err("a missing module is not current");
		assert!(error.to_string().contains("out of date"), "{error}");

		plugin
			.run_command(&context, "routes", &[])
			.expect("the module writes");

		plugin
			.run_command(&context, "routes", &["--check".to_owned()])
			.expect("the written module is current");

		let error = plugin
			.run_command(&context, "routes", &["--bogus".to_owned()])
			.expect_err("unknown arguments are rejected");
		assert!(error.to_string().contains("--bogus"), "{error}");
	}

	#[test]
	fn custom_output_paths_place_the_module() {
		let root = temp_root("custom-output");
		let plugin = RouterPlugin::default();
		plugin
			.configure(json!({ "output": "src/generated/routes.rs" }))
			.unwrap();
		let mut context = PluginGenerateContext::new(&root, leptoskit_config::Platform::Web, false);
		plugin.generate(&mut context).expect("the module generates");
		assert_eq!(
			context.outputs(),
			[root.join("src/generated/routes.rs")],
			"the custom output is recorded"
		);
		assert!(root.join("src/generated/routes.rs").is_file());
	}
}
