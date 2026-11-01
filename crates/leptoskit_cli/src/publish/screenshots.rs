//! `leptoskit publish screenshots`: every store's screenshots, captured
//! from the app with the same device control `leptoskit device` uses.
//!
//! `store/screenshots/screenshots.toml` names the store sizes, each the
//! folder its screenshots go in, and the screens to capture. A browser size
//! opens each screen's route in headless Chrome at an exact viewport; a
//! simulator or Android size launches the installed app, takes the screen's
//! steps to reach it, and captures the app.
//!
//! ```toml
//! url = "http://127.0.0.1:3000"   # the app, as `leptoskit dev` serves it
//!
//! [devices.chrome-1280x800]
//! viewport = [1280, 800]
//!
//! [devices."iphone-6.9"]  # quoted: the name has a dot
//! simulator = "iPhone 17 Pro Max"
//!
//! [[screenshot]]
//! name = "library"
//! route = "/library"
//! steps = [{ action = "tap", locator = { by = "text", value = "Library" } }]
//! ```

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_testing::control::Browser;
use leptoskit_testing::control::Document;
use leptoskit_testing::control::Locator;
use leptoskit_testing::control::SCHEMA_VERSION;
use leptoskit_testing::control::Step;
use leptoskit_testing::control::Target;
use leptoskit_testing::control::TargetKind;
use leptoskit_testing::control::inventory;
use leptoskit_testing::native::Application;
use serde::Deserialize;

/// Where the plan lives, and the screenshots go.
pub const DIR: &str = "store/screenshots";

/// `store/screenshots/screenshots.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
	/// Where browser sizes open routes: the app, as `leptoskit dev` serves
	/// it.
	#[serde(default = "default_url")]
	pub url: String,
	/// Each store size, by the folder its screenshots go in.
	pub devices: BTreeMap<String, Device>,
	/// The screens, in the order the stores show them.
	#[serde(rename = "screenshot", default)]
	pub screenshots: Vec<Shot>,
}

fn default_url() -> String {
	"http://127.0.0.1:3000".to_owned()
}

/// One store size: a browser viewport, a simulator, or an Android device.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
	/// A browser viewport, `[width, height]` in CSS pixels.
	pub viewport: Option<[u32; 2]>,
	/// Device pixels per CSS pixel, for a browser viewport; 1 when unset.
	pub scale: Option<f64>,
	/// Lay the browser viewport out as a phone's, with touch.
	#[serde(default)]
	pub mobile: bool,
	/// An iOS simulator, by name or UDID; booted.
	pub simulator: Option<String>,
	/// An Android device or running emulator, by adb serial.
	pub android: Option<String>,
}

/// One screen to capture.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shot {
	/// Names the file: `01-<name>.png`.
	pub name: String,
	/// The route browser sizes open; `/` when unset.
	#[serde(default = "default_route")]
	pub route: String,
	/// What a native app does after it launches to reach this screen, as
	/// control document steps.
	#[serde(default)]
	pub steps: Vec<Step>,
	/// A control that must be visible before the screenshot.
	pub wait_for: Option<Locator>,
	/// Only these sizes; every size when empty.
	#[serde(default)]
	pub devices: Vec<String>,
}

fn default_route() -> String {
	"/".to_owned()
}

/// One screenshot: the document that takes it, and where it goes.
#[derive(Debug)]
pub struct Capture {
	/// The store size's folder.
	pub device: String,
	/// The file inside it.
	pub file: String,
	/// The pixel size it must have, when the plan fixes one.
	pub size: Option<(u32, u32)>,
	pub document: Document,
}

impl Plan {
	/// Reads `store/screenshots/screenshots.toml` under `root`.
	///
	/// # Errors
	///
	/// When it is missing or can't be parsed.
	pub fn load(root: &Path) -> Result<Self> {
		let path = root.join(DIR).join("screenshots.toml");
		let text = std::fs::read_to_string(&path).with_context(|| {
			format!(
				"could not read {}; it names the store sizes and the screens to capture",
				path.display()
			)
		})?;
		toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))
	}

	/// Every screenshot's document, sizes in order, then screens.
	///
	/// # Errors
	///
	/// When a size names no device, or a simulator that isn't attached.
	pub fn captures(
		&self,
		package_id: &str,
		devices: &inventory::Inventory,
		artifacts: &Path,
	) -> Result<Vec<Capture>> {
		let mut captures = Vec::new();

		for (folder, device) in &self.devices {
			let shots = self
				.screenshots
				.iter()
				.filter(|shot| shot.devices.is_empty() || shot.devices.contains(folder));

			for (index, shot) in shots.enumerate() {
				let (target, mut steps, size) = match device {
					Device {
						viewport: Some([width, height]),
						..
					} => {
						let scale = device.scale.unwrap_or(1.0);
						let target = Target {
							url: Some(format!("{}{}", self.url.trim_end_matches('/'), shot.route)),
							browser: Some(Browser::Chrome),
							capabilities: BTreeMap::from([(
								"goog:chromeOptions".to_owned(),
								serde_json::json!({ "mobileEmulation": { "deviceMetrics": {
									"width": width, "height": height, "pixelRatio": scale,
									"mobile": device.mobile, "touch": device.mobile,
								}}}),
							)]),
							..target(TargetKind::Web, None, None, artifacts)
						};
						#[allow(
							clippy::cast_possible_truncation,
							clippy::cast_sign_loss,
							reason = "screen sizes are small and positive"
						)]
						let pixels = |length: u32| (f64::from(length) * scale).round() as u32;
						(target, Vec::new(), Some((pixels(*width), pixels(*height))))
					}
					Device {
						simulator: Some(simulator),
						..
					} => {
						let udid = devices
							.devices
							.iter()
							.find(|entry| {
								entry.kind == inventory::DeviceKind::IosSimulator
									&& (&entry.id == simulator || &entry.name == simulator)
							})
							.map(|entry| entry.id.clone())
							.with_context(|| {
								format!(
									"no simulator {simulator:?} for {folder}; `leptoskit device \
									 list` shows them"
								)
							})?;
						let target = target(
							TargetKind::IosSimulator,
							Some(udid),
							Some(package_id),
							artifacts,
						);
						(target, shot.steps.clone(), None)
					}
					Device {
						android: Some(serial),
						..
					} => {
						let target = target(
							TargetKind::Android,
							Some(serial.clone()),
							Some(package_id),
							artifacts,
						);
						(target, shot.steps.clone(), None)
					}
					_ => {
						anyhow::bail!(
							"devices.{folder} needs a viewport, a simulator, or an android serial"
						)
					}
				};

				if let Some(locator) = &shot.wait_for {
					steps.push(Step::ExpectVisible {
						locator: locator.clone(),
					});
				}

				steps.push(Step::Screenshot {
					label: shot.name.clone(),
				});
				captures.push(Capture {
					device: folder.clone(),
					file: format!("{:02}-{}.png", index + 1, shot.name),
					size,
					document: Document {
						schema_version: SCHEMA_VERSION,
						target,
						steps,
					},
				});
			}
		}

		Ok(captures)
	}
}

fn target(kind: TargetKind, device: Option<String>, app: Option<&str>, artifacts: &Path) -> Target {
	Target {
		kind,
		device,
		application: app.map(|app| Application::Identifier(app.to_owned())),
		url: None,
		browser: None,
		endpoint: None,
		capabilities: BTreeMap::new(),
		headless: true,
		startup_seconds: 300,
		command_seconds: 30,
		wait_seconds: 30,
		artifacts: artifacts.to_path_buf(),
	}
}

/// The pixel size of a PNG.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
	let header = bytes.get(16..24)?;
	Some((
		u32::from_be_bytes(header[..4].try_into().ok()?),
		u32::from_be_bytes(header[4..].try_into().ok()?),
	))
}

/// `leptoskit publish screenshots`: captures every screenshot, replacing the
/// ones an earlier run saved. With `dry_run`, lists them instead.
///
/// # Errors
///
/// When the plan can't be read, or any screenshot fails.
pub fn run(root: &Path, package_id: &str, dry_run: bool) -> Result<()> {
	let plan = Plan::load(root)?;
	let unknown: Vec<&String> = plan
		.devices
		.keys()
		.filter(|folder| {
			!super::images::TARGETS
				.iter()
				.any(|target| &target.folder == folder)
		})
		.collect();
	anyhow::ensure!(
		unknown.is_empty(),
		"`leptoskit publish` uploads only the store sizes it knows, so it would refuse {}; name \
		 each size one of the folders in the publishing guide, such as iphone-6.3, ipad-13, \
		 android-phone, or chrome-1280x800",
		unknown
			.iter()
			.map(|folder| format!("devices.{folder}"))
			.collect::<Vec<_>>()
			.join(", ")
	);
	let artifacts = root.join(".leptoskit/publish/screenshots");
	let devices = if dry_run {
		// A dry run lists the files, so it needs no simulator booted.
		inventory::Inventory {
			schema_version: inventory::SCHEMA_VERSION,
			devices: plan
				.devices
				.values()
				.filter_map(|device| device.simulator.clone())
				.map(|simulator| {
					inventory::Device {
						id: simulator.clone(),
						name: simulator,
						kind: inventory::DeviceKind::IosSimulator,
						state: String::new(),
					}
				})
				.collect(),
		}
	} else if plan
		.devices
		.values()
		.any(|device| device.simulator.is_some())
	{
		inventory::list()
	} else {
		inventory::Inventory {
			schema_version: inventory::SCHEMA_VERSION,
			devices: Vec::new(),
		}
	};

	let captures = plan.captures(package_id, &devices, &artifacts)?;

	if dry_run {
		for capture in &captures {
			println!("{DIR}/{}/{}", capture.device, capture.file);
		}

		println!(
			"\n{} screenshots on {} sizes; nothing was captured.",
			captures.len(),
			plan.devices.len()
		);

		return Ok(());
	}

	// What an earlier run saved goes, so a removed screen doesn't linger.
	for folder in plan.devices.keys() {
		let dir = root.join(DIR).join(folder);

		for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
			let name = entry.file_name().to_string_lossy().into_owned();
			let numbered = name.len() > 3
				&& name.as_bytes()[..2].iter().all(u8::is_ascii_digit)
				&& name.as_bytes()[2] == b'-';

			if numbered
				&& Path::new(&name)
					.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
			{
				std::fs::remove_file(entry.path())?;
			}
		}
	}

	let mut failed = Vec::new();

	for capture in &captures {
		let destination = root.join(DIR).join(&capture.device).join(&capture.file);
		let outcome = leptoskit_testing::control::run(&capture.document);
		let saved: Option<PathBuf> = outcome
			.steps
			.last()
			.and_then(|step| step.artifact.clone())
			.filter(|_| outcome.passed);

		if let Some(saved) = saved {
			std::fs::create_dir_all(destination.parent().expect("in a folder"))?;
			std::fs::copy(&saved, &destination)?;
			let size = png_size(&std::fs::read(&destination)?);

			match (capture.size, size) {
				(Some(expected), Some(actual)) if expected != actual => {
					crate::ui::warning(format_args!(
						"{} is {}×{}, not {}×{}",
						destination.display(),
						actual.0,
						actual.1,
						expected.0,
						expected.1
					));
				}

				_ => println!("saved {DIR}/{}/{}", capture.device, capture.file),
			}
		} else {
			let why = outcome
				.error
				.or_else(|| outcome.steps.iter().find_map(|step| step.error.clone()))
				.unwrap_or_else(|| "no screenshot was saved".to_owned());
			failed.push(format!("{}/{}: {why}", capture.device, capture.file));
		}
	}

	anyhow::ensure!(
		failed.is_empty(),
		"{} of {} screenshots failed:\n- {}",
		failed.len(),
		captures.len(),
		failed.join("\n- ")
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn commonplace_s_plan_becomes_control_documents() {
		let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/commonplace");
		let plan = Plan::load(&example).unwrap();
		let devices = inventory::Inventory {
			schema_version: inventory::SCHEMA_VERSION,
			devices: vec![inventory::Device {
				id: "SIM-UDID".to_owned(),
				name: "iPhone 17 Pro Max".to_owned(),
				kind: inventory::DeviceKind::IosSimulator,
				state: "Booted".to_owned(),
			}],
		};
		let captures = plan
			.captures(
				"com.leptoskit.commonplace",
				&devices,
				Path::new(".leptoskit/publish/screenshots"),
			)
			.unwrap();
		let summary: Vec<serde_json::Value> = captures
			.iter()
			.map(|capture| {
				serde_json::json!({
					"file": format!("{}/{}", capture.device, capture.file),
					"size": capture.size,
					"document": capture.document,
				})
			})
			.collect();
		insta::assert_json_snapshot!(summary);

		let error = plan
			.captures(
				"com.leptoskit.commonplace",
				&inventory::Inventory {
					schema_version: inventory::SCHEMA_VERSION,
					devices: Vec::new(),
				},
				Path::new("out"),
			)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("no simulator \"iPhone 17 Pro Max\""),
			"{error}"
		);
	}

	#[test]
	fn png_sizes_come_from_the_header() {
		let mut png = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut png, 3, 2);
			encoder.set_color(png::ColorType::Rgba);
			let mut writer = encoder.write_header().unwrap();
			writer.write_image_data(&[0; 3 * 2 * 4]).unwrap();
		}
		assert_eq!(png_size(&png), Some((3, 2)));
		assert_eq!(png_size(b"short"), None);
	}
}
