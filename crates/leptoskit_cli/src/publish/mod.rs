//! `leptoskit publish`: from source to each store's review queue.
//!
//! Publishing has two steps on every store. **Upload** puts a build where
//! the store keeps it as a draft; `--submit` also sends it for review or
//! release, which reaches users, so nothing does without it. `--dry-run`
//! builds and checks everything and contacts no store. Every run ends with
//! the steps the stores' APIs can't take, which a person, or an agent with
//! `leptoskit skill`, finishes in each dashboard.

pub mod app_store;
pub mod chrome;
pub mod edge;
pub mod firefox;
pub mod google;
pub mod http;
pub mod listing;
pub mod macos;
pub mod play;
pub mod screenshots;
pub mod secrets;
pub mod steps;
pub mod tools;

use std::io::IsTerminal as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

use self::listing::Store;
use self::secrets::Credentials;
use self::steps::Field;
use self::steps::Report;
use self::steps::Status;
use self::steps::Step;
use self::steps::Who;

/// `leptoskit publish`'s arguments.
#[derive(Debug, Clone, clap::Args)]
#[allow(clippy::struct_excessive_bools, reason = "each is a command-line flag")]
pub struct PublishArgs {
	/// The stores: `chrome`, `edge`, `firefox`, `app_store`, `macos`, and
	/// `play`; every store `[publish]` configures when omitted.
	/// `credentials` lists the credentials each configured store needs, and
	/// `screenshots` captures the stores' screenshots instead.
	pub stores: Vec<String>,
	/// Build and check everything, and contact no store.
	#[arg(long)]
	pub dry_run: bool,
	/// Also send the upload for review or release, which reaches users.
	#[arg(long)]
	pub submit: bool,
	/// On the Chrome Web Store, keep an approved version until it is
	/// published by hand.
	#[arg(long)]
	pub staged: bool,
	/// Don't ask before submitting; for scripts.
	#[arg(long)]
	pub yes: bool,
	/// Print the report as JSON.
	#[arg(long)]
	pub json: bool,
	/// The app stores' build number (`CFBundleVersion`, `versionCode`):
	/// `auto`, one more than the store's highest, or a number. The macOS
	/// download keeps the app's own unless a number is given.
	#[arg(long, default_value = "auto")]
	pub build_number: BuildNumber,
	/// A TOML file of credentials, by their environment variables' names,
	/// outside the project.
	#[arg(long)]
	pub credentials: Option<PathBuf>,
}

/// Where the report and the store-ready files go.
const OUTPUT: &str = ".leptoskit/publish";

/// The stores that publish the browser extension's packages.
const EXTENSION_STORES: [&str; 3] = ["chrome", "edge", "firefox"];

/// An app store's build number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildNumber {
	/// One more than the store's highest.
	Auto,
	Number(u64),
}

impl std::str::FromStr for BuildNumber {
	type Err = String;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		if text == "auto" {
			return Ok(Self::Auto);
		}
		match text.parse() {
			Ok(number) if number > 0 => Ok(Self::Number(number)),
			_ => Err("a build number is `auto` or a whole number above 0".to_owned()),
		}
	}
}

/// A credential a store needs.
struct Needed {
	store: &'static str,
	name: &'static str,
	how: &'static str,
}

const NEEDED: [Needed; 17] = [
	Needed {
		store: "chrome",
		name: chrome::CREDENTIAL,
		how: chrome::HOW,
	},
	Needed {
		store: "edge",
		name: edge::CLIENT_ID,
		how: edge::HOW,
	},
	Needed {
		store: "edge",
		name: edge::API_KEY,
		how: edge::HOW,
	},
	Needed {
		store: "firefox",
		name: firefox::ISSUER,
		how: firefox::HOW,
	},
	Needed {
		store: "firefox",
		name: firefox::SECRET,
		how: firefox::HOW,
	},
	Needed {
		store: "app_store",
		name: app_store::ISSUER,
		how: app_store::HOW,
	},
	Needed {
		store: "app_store",
		name: app_store::KEY_ID,
		how: app_store::HOW,
	},
	Needed {
		store: "app_store",
		name: app_store::KEY,
		how: app_store::HOW,
	},
	Needed {
		store: "macos",
		name: macos::IDENTITY,
		how: macos::HOW,
	},
	Needed {
		store: "macos",
		name: app_store::ISSUER,
		how: app_store::HOW,
	},
	Needed {
		store: "macos",
		name: app_store::KEY_ID,
		how: app_store::HOW,
	},
	Needed {
		store: "macos",
		name: app_store::KEY,
		how: app_store::HOW,
	},
	Needed {
		store: "play",
		name: play::CREDENTIAL,
		how: play::HOW,
	},
	Needed {
		store: "play",
		name: play::KEYSTORE,
		how: play::KEYSTORE_HOW,
	},
	Needed {
		store: "play",
		name: play::KEYSTORE_PASSWORD,
		how: play::KEYSTORE_HOW,
	},
	Needed {
		store: "play",
		name: play::KEY_ALIAS,
		how: play::KEYSTORE_HOW,
	},
	Needed {
		store: "play",
		name: play::KEY_PASSWORD,
		how: play::KEYSTORE_HOW,
	},
];

/// An API's address: the store's own, or an override for a proxy or a
/// test server, such as `LEPTOSKIT_PUBLISH_CHROME_API`.
fn api(variable: &str, default: &str) -> String {
	std::env::var(variable).unwrap_or_else(|_| default.to_owned())
}

/// `leptoskit publish`.
///
/// # Errors
///
/// When the configuration, a check, a credential, or a store fails.
pub fn run(args: &PublishArgs) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let credentials = Credentials::load(args.credentials.as_deref(), &root)?;
	if args.stores.iter().any(|store| store == "screenshots") {
		return screenshots::run(&root, &config.native.package_id, args.dry_run);
	}
	let configured = config.publish.stores();
	anyhow::ensure!(
		!configured.is_empty(),
		"no store is configured: add a [publish] table for a store, such as [publish.chrome] or \
		 [publish.play], to leptoskit.toml"
	);
	if args.stores.iter().any(|store| store == "credentials") {
		print!("{}", list_credentials(&configured, &credentials));
		return Ok(());
	}
	let stores: Vec<&str> = if args.stores.is_empty() {
		configured.clone()
	} else {
		for store in &args.stores {
			anyhow::ensure!(
				configured.contains(&store.as_str()),
				"`{store}` isn't configured: add [publish.{store}] to leptoskit.toml"
			);
		}
		configured
			.iter()
			.copied()
			.filter(|store| args.stores.iter().any(|named| named == store))
			.collect()
	};
	if !args.dry_run {
		for needed in NEEDED
			.iter()
			.filter(|needed| stores.contains(&needed.store))
		{
			credentials.require(needed.name, needed.how)?;
		}
	}

	let store = Store::load(&root)?;
	let extension = if stores.iter().any(|store| EXTENSION_STORES.contains(store)) {
		Some(Extension::build()?)
	} else {
		None
	};
	let (name, version) = match &extension {
		Some(extension) => {
			(
				extension.manifest["name"]
					.as_str()
					.unwrap_or_default()
					.to_owned(),
				extension.manifest["version"]
					.as_str()
					.unwrap_or_default()
					.to_owned(),
			)
		}
		None => (display_name(&config), apple_version(&config)),
	};
	let submit = args.submit && !args.dry_run && confirm(args, &name, &version, &stores)?;
	let http = http::Http::default();
	let session = Session {
		args,
		root: &root,
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &tools::System,
		extension: extension.as_ref(),
		name,
		version,
		submit,
	};
	let mut report = Report::default();
	for store_name in &stores {
		let (summary, steps) = session.publish(store_name)?;
		report.steps.extend(steps);
		report.stores.push(Status {
			store: (*store_name).to_owned(),
			summary,
		});
	}

	let written = report.write(&root.join(OUTPUT))?;
	if args.json {
		println!("{}", serde_json::to_string_pretty(&report)?);
	} else {
		print!("{}", report.text());
		println!(
			"\nWrote these steps to {} (and next-steps.json).",
			written.display()
		);
	}
	Ok(())
}

/// The browser extension's release build, which the extension stores
/// upload.
struct Extension {
	built: crate::extension::Built,
	manifest: serde_json::Value,
}

impl Extension {
	fn build() -> Result<Self> {
		let built = crate::extension::build(true, None)?;
		let manifest = {
			let chromium = built
				.packages
				.iter()
				.find(|package| package.browser == leptoskit_config::Browser::Chromium)
				.or_else(|| built.packages.first())
				.context("the build made no package")?;
			serde_json::from_slice(&std::fs::read(chromium.dir.join("manifest.json"))?)?
		};
		Ok(Self { built, manifest })
	}
}

/// The app's name, as people see it.
fn display_name(config: &LeptoskitConfig) -> String {
	config
		.extension
		.as_ref()
		.and_then(|extension| extension.name.clone())
		.or_else(|| config.auth.as_ref().and_then(|auth| auth.app_name.clone()))
		.unwrap_or_else(|| config.app.name.clone())
}

/// `app.version` as Apple's version: up to three integers, so a
/// pre-release's label is dropped.
fn apple_version(config: &LeptoskitConfig) -> String {
	let version = &config.app.version;
	format!("{}.{}.{}", version.major, version.minor, version.patch)
}

/// Everything one run's stores share.
struct Session<'a> {
	args: &'a PublishArgs,
	root: &'a Path,
	config: &'a LeptoskitConfig,
	credentials: &'a Credentials,
	store: &'a Store,
	http: &'a http::Http,
	runner: &'a dyn tools::Runner,
	extension: Option<&'a Extension>,
	name: String,
	version: String,
	submit: bool,
}

impl Session<'_> {
	fn extension(&self) -> &Extension {
		self.extension
			.expect("the extension stores build the extension first")
	}

	/// A browser family's zip.
	fn package(&self, browser: leptoskit_config::Browser) -> Result<PathBuf> {
		self.extension()
			.built
			.packages
			.iter()
			.find(|package| package.browser == browser)
			.and_then(|package| package.zip.clone())
			.with_context(|| {
				format!("the build made no {browser} package; add it to [extension].browsers")
			})
	}

	fn checked(&self) -> String {
		format!(
			"{} {} is built and checked; nothing was uploaded.",
			self.name, self.version
		)
	}

	fn chrome(&self) -> Result<(String, Vec<Step>)> {
		let settings = self.config.publish.chrome.as_ref().expect("configured");
		let zip = self.package(leptoskit_config::Browser::Chromium)?;
		let summary = if self.args.dry_run {
			self.checked()
		} else {
			let account = self.credentials.require(chrome::CREDENTIAL, chrome::HOW)?;
			let client = chrome::Chrome::connect(
				self.http,
				&api("LEPTOSKIT_PUBLISH_CHROME_API", chrome::API),
				&api("LEPTOSKIT_PUBLISH_GOOGLE_OAUTH", chrome::OAUTH),
				&account,
				&settings.publisher_id,
				&settings.item_id,
			)?;
			client.upload(std::fs::read(&zip)?)?;
			if self.submit {
				let state = client.submit(self.args.staged)?;
				format!(
					"{} {} is submitted for review ({state}).",
					self.name, self.version
				)
			} else {
				format!("{} {} is uploaded as a draft.", self.name, self.version)
			}
		};
		Ok((
			summary,
			chrome_steps(
				self.config,
				self.store,
				&self.extension().manifest,
				self.submit,
			),
		))
	}

	fn edge(&self) -> Result<(String, Vec<Step>)> {
		let settings = self.config.publish.edge.as_ref().expect("configured");
		let zip = self.package(leptoskit_config::Browser::Chromium)?;
		let summary = if self.args.dry_run {
			self.checked()
		} else {
			let client = edge::Edge::new(
				self.http,
				&api("LEPTOSKIT_PUBLISH_EDGE_API", edge::API),
				&settings.product_id,
				self.credentials.require(edge::CLIENT_ID, edge::HOW)?,
				self.credentials.require(edge::API_KEY, edge::HOW)?,
			);
			client.upload(std::fs::read(&zip)?)?;
			if self.submit {
				let notes = self
					.store
					.release_notes
					.clone()
					.unwrap_or_else(|| format!("Version {}.", self.version));
				client.submit(&notes)?;
				format!(
					"{} {} is submitted for certification.",
					self.name, self.version
				)
			} else {
				format!("{} {} is uploaded as a draft.", self.name, self.version)
			}
		};
		Ok((summary, edge_steps(self.config, self.store, self.submit)))
	}

	fn firefox(&self) -> Result<(String, Vec<Step>)> {
		let settings = self.config.publish.firefox.clone().unwrap_or_default();
		let guid = self
			.config
			.extension
			.as_ref()
			.and_then(|extension| extension.firefox.as_ref())
			.map(|firefox| firefox.id.clone())
			.context("[extension.firefox] needs the add-on's id")?;
		self.fits("firefox")?;
		let zip = self.package(leptoskit_config::Browser::Firefox)?;
		let source = self
			.extension()
			.built
			.source
			.clone()
			.context("the build wrote no Firefox source archive")?;
		let mut exists = true;
		let summary = if self.args.dry_run {
			self.checked()
		} else {
			let client = firefox::Firefox::new(
				self.http,
				&api("LEPTOSKIT_PUBLISH_FIREFOX_API", firefox::API),
				&guid,
				self.credentials.require(firefox::ISSUER, firefox::HOW)?,
				self.credentials.require(firefox::SECRET, firefox::HOW)?,
			);
			let upload = client.upload(&std::fs::read(&zip)?, settings.channel.name())?;
			let summary = if self.submit {
				match client.submit(&upload, &std::fs::read(&source)?) {
					Ok(_) => {
						format!(
							"{} {} is submitted ({} channel).",
							self.name,
							self.version,
							settings.channel.name()
						)
					}
					Err(error) if error.to_string().contains("has no add-on") => {
						exists = false;
						format!(
							"{} {} passed validation; the add-on needs its first submission on \
							 the site.",
							self.name, self.version
						)
					}
					Err(error) => return Err(error),
				}
			} else {
				format!(
					"{} {} passed addons.mozilla.org's validation; nothing is submitted yet.",
					self.name, self.version
				)
			};
			// A listed add-on's text comes from `store/`; an unlisted one has
			// no listing.
			if exists && settings.channel == leptoskit_config::publish::FirefoxChannel::Listed {
				exists = client.listing(
					self.store.listing.name.as_deref(),
					self.store.summary.as_deref(),
					self.store.description.as_deref(),
				)?;
			}
			summary
		};
		Ok((
			summary,
			firefox_steps(
				&guid,
				&settings,
				self.store,
				self.submit,
				exists,
				&zip,
				&source,
			),
		))
	}
}

impl Session<'_> {
	fn app_store(&self) -> Result<(String, Vec<Step>)> {
		let settings = self.config.publish.app_store.as_ref().expect("configured");
		let app = &settings.app_id;
		let team = settings
			.team(self.config.native.apple_team_id.as_deref())
			.context("[publish.app_store] needs team_id")?;
		self.fits("app_store")?;
		let version = apple_version(self.config);
		let out = self.root.join(OUTPUT).join("ios");
		std::fs::create_dir_all(&out)?;
		std::fs::write(
			out.join("ExportOptions.plist"),
			app_store::export_options(team),
		)?;
		let info_plist =
			crate::native::host_dir(self.root, crate::native::Target::Ios).join("Info.plist");
		let mut uploaded = None;
		let mut beta_review = None;
		let summary = if self.args.dry_run {
			let build = match self.args.build_number {
				BuildNumber::Auto => "one more than App Store Connect's highest".to_owned(),
				BuildNumber::Number(number) => number.to_string(),
			};
			format!(
				"{} {version} is checked; an upload archives it for team {team}, and its build \
				 number will be {build}. Nothing was built or uploaded.",
				self.name
			)
		} else {
			let key = app_store::Key::new(
				self.credentials
					.require(app_store::ISSUER, app_store::HOW)?,
				self.credentials
					.require(app_store::KEY_ID, app_store::HOW)?,
				&self.credentials.require(app_store::KEY, app_store::HOW)?,
			)?;
			let client = app_store::AppStore::new(
				self.http,
				&api("LEPTOSKIT_PUBLISH_APPLE_API", app_store::API),
				&key,
			);
			let build = match self.args.build_number {
				BuildNumber::Auto => client.highest_build(app)?.map_or(1, |highest| highest + 1),
				BuildNumber::Number(number) => number,
			};
			crate::native::run(crate::native::NativeCommand::Prepare {
				target: crate::native::Target::Ios,
				release: true,
			})?;
			for stale in ["App.xcarchive", "export"] {
				let stale = out.join(stale);
				if stale.exists() {
					std::fs::remove_dir_all(&stale)?;
				}
			}
			let file = key.file(&out)?;
			self.runner.run(&app_store::archive(
				self.root, &out, team, &version, build, &file,
			))?;
			self.runner.run(&app_store::export(&out, &file))?;
			drop(file);
			let id = client.wait_for_build(app, &version, build)?;
			uploaded = Some(build);
			let groups = client.add_to_groups(app, &id, &settings.testflight_groups)?;
			if groups.iter().any(|group| !group.internal)
				&& let Err(error) = client.submit_for_beta_review(&id)
			{
				beta_review = Some(format!("{error:#}"));
			}
			let tested = if groups.is_empty() {
				String::new()
			} else {
				format!(
					" for {}",
					groups
						.iter()
						.map(|group| group.name.as_str())
						.collect::<Vec<_>>()
						.join(", ")
				)
			};
			// The listing's text goes up with every upload, on the version in
			// preparation, which nobody sees until it is submitted.
			let (version_id, first) = client.version(app, &version)?;
			let text = app_store_text(self.store);
			if !text.is_empty() {
				client.localize(&version_id, "en-US", &serde_json::Value::Object(text))?;
			}
			if let Some(url) = &self.store.listing.privacy_policy {
				client.privacy_policy(app, url)?;
			}
			if self.submit {
				if !first {
					let notes = self
						.store
						.release_notes
						.clone()
						.unwrap_or_else(|| format!("Version {version}."));
					client.localize(
						&version_id,
						"en-US",
						&serde_json::json!({ "whatsNew": notes }),
					)?;
				}
				client.attach(&version_id, &id)?;
				let state = client.submit_for_review(app, &version_id)?;
				format!(
					"{} {version} (build {build}) is in TestFlight{tested}, and submitted for App \
					 Review ({state}).",
					self.name
				)
			} else {
				format!(
					"{} {version} (build {build}) is in TestFlight{tested}.",
					self.name
				)
			}
		};
		Ok((
			summary,
			app_store_steps(&AppStoreRun {
				app,
				version: &version,
				build: uploaded,
				root: self.root,
				info_plist: &info_plist,
				encryption_answered: encryption_answered(&info_plist),
				beta_review: beta_review.as_deref(),
				submitted: self.submit,
				store: self.store,
			}),
		))
	}
}

impl Session<'_> {
	/// The relative path of a file under the project, for messages.
	fn relative(&self, path: &Path) -> String {
		path.strip_prefix(self.root)
			.unwrap_or(path)
			.display()
			.to_string()
	}

	fn macos(&self) -> Result<(String, Vec<Step>)> {
		let settings = self.config.publish.macos.clone().unwrap_or_default();
		let out = self.root.join(OUTPUT).join("macos");
		let version = apple_version(self.config);
		let entitlements = out.join("SafariExtension.entitlements");
		let plan = macos::Plan {
			built: &self.root.join(".leptoskit/macos/App.app"),
			out: &out,
			name: &self.name,
			version: &version,
			build: match self.args.build_number {
				BuildNumber::Auto => None,
				BuildNumber::Number(number) => Some(number),
			},
			identity: "",
			safari_entitlements: crate::native::safari::configured(self.config)
				.then_some(entitlements.as_path()),
		};
		let artifacts = plan.artifacts();
		let (dmg, zip) = (self.relative(&artifacts.dmg), self.relative(&artifacts.zip));
		let tag = format!("v{}", self.config.app.version);
		let mut released = false;
		let summary = if self.args.dry_run {
			format!(
				"{} {version} is checked; an upload signs it with your Developer ID, notarizes \
				 it, and writes {dmg} and {zip}. Nothing was built or notarized.",
				self.name
			)
		} else {
			let identity = self.credentials.require(macos::IDENTITY, macos::HOW)?;
			let key = app_store::Key::new(
				self.credentials
					.require(app_store::ISSUER, app_store::HOW)?,
				self.credentials
					.require(app_store::KEY_ID, app_store::HOW)?,
				&self.credentials.require(app_store::KEY, app_store::HOW)?,
			)?;
			crate::native::run(crate::native::NativeCommand::Build {
				target: crate::native::Target::Macos,
				release: true,
			})?;
			std::fs::create_dir_all(&out)?;
			std::fs::write(&entitlements, crate::native::safari::ENTITLEMENTS)?;
			let file = key.file(&out)?;
			let artifacts = macos::distribute(
				self.runner,
				&macos::Plan {
					identity: identity.expose(),
					..plan
				},
				&file,
			)?;
			drop(file);
			match &settings.github_release {
				Some(repository) if self.submit => {
					let notes = self
						.store
						.release_notes
						.clone()
						.unwrap_or_else(|| format!("{} {version}.", self.name));
					macos::release(
						self.runner,
						repository,
						&tag,
						&format!("{} {version}", self.name),
						&notes,
						&[&artifacts.dmg, &artifacts.zip],
					)?;
					released = true;
					format!(
						"{} {version} is notarized, and attached to the release {tag} of \
						 {repository}.",
						self.name
					)
				}
				_ => {
					format!(
						"{} {version} is signed, notarized, and stapled: {dmg} and {zip}.",
						self.name
					)
				}
			}
		};
		let steps = match &settings.github_release {
			_ if released => Vec::new(),
			Some(repository) => {
				vec![step(
					"macos",
					Who::Person,
					&format!(
						"Decide to release it, then run `leptoskit publish macos --submit`, which \
						 attaches {dmg} and {zip} to the release {tag} of {repository}."
					),
					None,
					Vec::new(),
					"a release on GitHub reaches everyone who downloads from it",
					"the release lists both files",
				)]
			}
			None => {
				vec![step(
					"macos",
					Who::Agent,
					"Put the disk image on the page people download the app from.",
					None,
					vec![
						field("Disk image", dmg),
						field("For updaters and scripts", zip),
					],
					"leptoskit doesn't host downloads; set [publish.macos].github_release to \
					 attach them to a GitHub release",
					"the page offers the new version",
				)]
			}
		};
		Ok((summary, steps))
	}
}

impl Session<'_> {
	fn play(&self) -> Result<(String, Vec<Step>)> {
		self.fits("play")?;
		let settings = self.config.publish.play.clone().unwrap_or_default();
		let track = settings.track.as_str();
		let package = &self.config.native.package_id;
		let android = crate::native::host_dir(self.root, crate::native::Target::Android);
		let script = android.join("app/build.gradle.kts");
		let text = std::fs::read_to_string(&script)
			.with_context(|| format!("could not read {}", script.display()))?;
		let problems = play::problems(&text, &script);
		anyhow::ensure!(
			problems.is_empty(),
			"Google Play can't take this app's bundle yet:\n- {}",
			problems.join("\n- ")
		);
		let notes = self.store.release_notes.as_deref();
		if let Some(notes) = notes {
			anyhow::ensure!(
				notes.chars().count() <= play::NOTES_LIMIT,
				"store/en-US/release_notes.md has {} characters; Google Play takes at most {}",
				notes.chars().count(),
				play::NOTES_LIMIT
			);
		}
		let version = self.config.app.version.to_string();
		let aab = play::bundle_path(&android);
		let mut exists = true;
		let mut rollout = play::Rollout::Draft;
		let summary = if self.args.dry_run {
			let code = match self.args.build_number {
				BuildNumber::Auto => "one more than the highest on Google Play".to_owned(),
				BuildNumber::Number(number) => number.to_string(),
			};
			format!(
				"{} {version} is checked; an upload builds an App Bundle signed with your upload \
				 key, numbered {code}, and puts it on the {track} track as a draft. Nothing was \
				 built or uploaded.",
				self.name
			)
		} else {
			let account = self.credentials.require(play::CREDENTIAL, play::HOW)?;
			let signing = play::Signing {
				keystore: self
					.credentials
					.require(play::KEYSTORE, play::KEYSTORE_HOW)?,
				keystore_password: self
					.credentials
					.require(play::KEYSTORE_PASSWORD, play::KEYSTORE_HOW)?,
				key_alias: self
					.credentials
					.require(play::KEY_ALIAS, play::KEYSTORE_HOW)?,
				key_password: self
					.credentials
					.require(play::KEY_PASSWORD, play::KEYSTORE_HOW)?,
			};
			let client = play::Play::connect(
				self.http,
				&api("LEPTOSKIT_PUBLISH_PLAY_API", play::API),
				&api("LEPTOSKIT_PUBLISH_GOOGLE_OAUTH", google::OAUTH),
				&account,
				package,
			)?;
			let highest = if let Some(edit) = client.edit()? {
				let highest = edit.highest_version_code();
				edit.discard();
				highest?
			} else {
				exists = false;
				None
			};
			let code = match self.args.build_number {
				BuildNumber::Auto => highest.map_or(1, |highest| highest + 1),
				BuildNumber::Number(number) => number,
			};
			crate::native::run(crate::native::NativeCommand::Prepare {
				target: crate::native::Target::Android,
				release: true,
			})?;
			if aab.exists() {
				std::fs::remove_file(&aab)?;
			}
			self.runner.run(&play::bundle(&android, code, &signing))?;
			match client.edit()? {
				Some(edit) if exists => {
					let uploaded = edit.upload(
						std::fs::read(&aab)
							.with_context(|| format!("Gradle wrote no {}", aab.display()))?,
					)?;
					anyhow::ensure!(
						uploaded == code,
						"Google Play read versionCode {uploaded} from the bundle, not {code}; \
						 check versionCode in {}",
						script.display()
					);
					if self.submit {
						rollout = match settings.rollout {
							Some(fraction) if track == "production" => {
								play::Rollout::Staged(fraction)
							}
							_ => play::Rollout::Everyone,
						};
					}
					if let (Some(short), Some(full)) =
						(&self.store.summary, &self.store.description)
					{
						let title = self.store.listing.name.as_deref().unwrap_or(&self.name);
						edit.listing("en-US", title, short, full)?;
					}
					edit.release(track, &play::release(&version, code, rollout, notes))?;
					edit.commit()?;
					play_summary(&self.name, &version, code, track, rollout)
				}
				_ => {
					exists = false;
					format!(
						"{} {version} (versionCode {code}) is built; Google Play has no app \
						 {package} yet, so its first bundle is uploaded by hand.",
						self.name
					)
				}
			}
		};
		Ok((
			summary,
			play_steps(&PlayRun {
				package,
				track,
				aab: &self.relative(&aab),
				exists,
				rollout,
				submitted: self.submit,
				store: self.store,
			}),
		))
	}
}

/// Where an upload to Google Play stands.
fn play_summary(
	name: &str,
	version: &str,
	code: u64,
	track: &str,
	rollout: play::Rollout,
) -> String {
	match rollout {
		play::Rollout::Draft => {
			format!(
				"{name} {version} (versionCode {code}) is on the {track} track as a draft release."
			)
		}
		play::Rollout::Everyone => {
			format!("{name} {version} (versionCode {code}) is released on the {track} track.")
		}
		play::Rollout::Staged(fraction) => {
			format!(
				"{name} {version} (versionCode {code}) is rolling out to {}% of the {track} \
				 track's users.",
				fraction * 100.0
			)
		}
	}
}

/// What one Google Play run did, for its steps.
struct PlayRun<'a> {
	package: &'a str,
	track: &'a str,
	aab: &'a str,
	exists: bool,
	rollout: play::Rollout,
	submitted: bool,
	store: &'a Store,
}

/// What Google Play's API can't do.
fn play_steps(run: &PlayRun<'_>) -> Vec<Step> {
	let console = "https://play.google.com/console".to_owned();
	let mut steps = Vec::new();
	if !run.exists {
		steps.push(step(
			"play",
			Who::Agent,
			&format!(
				"Create the app {} in Play Console, and upload its first App Bundle to the {} \
				 track.",
				run.package, run.track
			),
			Some(console.clone()),
			vec![field("App Bundle", run.aab)],
			"Google Play's API publishes to an app only after Play Console has its first bundle",
			"`leptoskit publish play` uploads instead",
		));
	}
	if run.submitted {
		if let play::Rollout::Staged(fraction) = run.rollout {
			steps.push(step(
				"play",
				Who::Person,
				&format!(
					"Watch the first {}% of users, then raise the rollout in Play Console, or \
					 halt it.",
					fraction * 100.0
				),
				Some(console),
				Vec::new(),
				"how far to roll out is the account holder's decision",
				"the release reaches 100%",
			));
		}
		return steps;
	}
	steps.extend([
		step(
			"play",
			Who::Agent,
			"Add the main store listing's images: the icon, the feature graphic, and phone \
			 screenshots.",
			Some(console.clone()),
			std::iter::once(field(
				"Screenshots",
				"store/screenshots/android-phone/*.png, made by `leptoskit publish screenshots`",
			))
			.chain(missing(&[
				(
					"Short description",
					run.store.summary.is_some(),
					"write it in store/en-US/summary.md, 80 characters at most",
				),
				(
					"Full description",
					run.store.description.is_some(),
					"write it in store/en-US/description.md",
				),
			]))
			.collect(),
			"leptoskit uploads the listing's title and descriptions with each upload, but not \
			 images yet",
			"Play Console's dashboard no longer lists the store listing as a task",
		),
		step(
			"play",
			Who::Agent,
			"Set the privacy policy, under App content.",
			Some(console.clone()),
			vec![field("Privacy policy", privacy_policy(run.store))],
			"the Play Developer API has no App content endpoint",
			"App content shows the privacy policy as done",
		),
		step(
			"play",
			Who::Person,
			"Answer App content's declarations: data safety, the content rating questionnaire, \
			 the target audience, and ads.",
			Some(console),
			Vec::new(),
			"they are legal declarations about the app, which only the account holder can make",
			"App content shows no task left",
		),
		step(
			"play",
			Who::Person,
			&format!(
				"Decide to release it, then run `leptoskit publish play --submit`, which releases \
				 the {} track (a staged rollout on production when `rollout` is set).",
				run.track
			),
			None,
			Vec::new(),
			"releasing reaches the track's users, which is the account holder's decision",
			"the track's release shows as available or in progress",
		),
	]);
	steps
}

impl Session<'_> {
	/// Fails when `store/` has text too long for `store`, before anything is
	/// built.
	fn fits(&self, store: &str) -> Result<()> {
		let problems = self.store.too_long(store, &self.name);
		anyhow::ensure!(problems.is_empty(), "{}", problems.join("; "));
		Ok(())
	}
}

/// The version's text App Store Connect takes from `store/`.
fn app_store_text(store: &Store) -> serde_json::Map<String, serde_json::Value> {
	let mut text = serde_json::Map::new();
	for (field, value) in [
		("description", store.description.clone()),
		("keywords", store.keywords()),
		("promotionalText", store.summary.clone()),
		("supportUrl", store.listing.support.clone()),
		("marketingUrl", store.listing.homepage.clone()),
	] {
		if let Some(value) = value {
			text.insert(field.to_owned(), serde_json::Value::String(value));
		}
	}
	text
}

/// Fields for the text `store/` is missing, so a step says what to write.
fn missing(fields: &[(&str, bool, &str)]) -> Vec<Field> {
	fields
		.iter()
		.filter(|(_, present, _)| !present)
		.map(|(label, _, hint)| field(label, format!("({hint})")))
		.collect()
}

/// Whether the app's `Info.plist` answers export compliance for every build
/// with `ITSAppUsesNonExemptEncryption`.
fn encryption_answered(plist: &Path) -> bool {
	plist::Value::from_file(plist).is_ok_and(|plist| {
		plist
			.as_dictionary()
			.is_some_and(|plist| plist.contains_key("ITSAppUsesNonExemptEncryption"))
	})
}

/// What one App Store run did, for its steps.
struct AppStoreRun<'a> {
	app: &'a str,
	version: &'a str,
	/// The build uploaded, when one was.
	build: Option<u64>,
	root: &'a Path,
	/// The app-owned `Info.plist`.
	info_plist: &'a Path,
	encryption_answered: bool,
	/// Why TestFlight's beta review refused the build, when it did.
	beta_review: Option<&'a str>,
	submitted: bool,
	store: &'a Store,
}

/// What App Store Connect's API can't do.
fn app_store_steps(run: &AppStoreRun<'_>) -> Vec<Step> {
	let app = format!("https://appstoreconnect.apple.com/apps/{}", run.app);
	let build = run
		.build
		.map_or_else(|| "each build".to_owned(), |build| format!("build {build}"));
	let info_plist = run
		.info_plist
		.strip_prefix(run.root)
		.unwrap_or(run.info_plist)
		.display()
		.to_string();
	let mut steps = Vec::new();
	if let Some(problem) = run.beta_review {
		steps.push(step(
			"app_store",
			Who::Agent,
			&format!("Submit {build} for TestFlight's beta review, so external testers get it."),
			Some(format!("{app}/testflight/ios")),
			vec![field("App Store Connect said", problem)],
			"App Store Connect refused it through the API, usually until export compliance is \
			 answered",
			"the build shows as waiting for beta review",
		));
	}
	if !run.encryption_answered {
		steps.push(step(
			"app_store",
			Who::Person,
			&format!("Answer the export compliance question for {build} in TestFlight."),
			Some(format!("{app}/testflight/ios")),
			vec![field(
				"To answer it once",
				format!(
					"set ITSAppUsesNonExemptEncryption in {info_plist}, and every later build \
					 carries the answer"
				),
			)],
			"it is a legal declaration about the app's encryption",
			"the build no longer shows Missing Compliance",
		));
	}
	if run.submitted {
		// App Review takes a version only once everything below is done.
		return steps;
	}
	let listing = &run.store.listing;
	steps.extend([
		step(
			"app_store",
			Who::Agent,
			&format!(
				"Add version {}'s screenshots for iPhone and iPad, on its App Store page.",
				run.version
			),
			Some(format!("{app}/distribution/ios/version/inflight")),
			std::iter::once(field(
				"Screenshots",
				"store/screenshots/iphone-6.9/*.png and store/screenshots/ipad-13/*.png, made by \
				 `leptoskit publish screenshots`",
			))
			.chain(missing(&[
				(
					"Description",
					run.store.description.is_some(),
					"write it in store/en-US/description.md",
				),
				(
					"Keywords",
					run.store.keywords().is_some(),
					"add keywords to store/listing.toml",
				),
				(
					"Support URL",
					listing.support.is_some(),
					"add support to store/listing.toml",
				),
			]))
			.collect(),
			"leptoskit uploads the version's description, keywords, promotional text, and \
			 addresses with each upload, but not screenshots yet",
			"`leptoskit publish app_store --submit` succeeds; App Review refuses a page without \
			 screenshots",
		),
		step(
			"app_store",
			Who::Agent,
			"Choose the category, in App Information.",
			Some(format!("{app}/distribution/info")),
			std::iter::once(field(
				"Category",
				listing.category.clone().unwrap_or_else(|| {
					"(choose one, and add category to store/listing.toml)".to_owned()
				}),
			))
			.chain(missing(&[(
				"Privacy Policy URL",
				listing.privacy_policy.is_some(),
				"add privacy_policy to store/listing.toml",
			)]))
			.collect(),
			"the category is chosen from App Store Connect's list; leptoskit uploads the privacy \
			 policy's address",
			"a submit succeeds",
		),
		step(
			"app_store",
			Who::Person,
			"Answer the App Privacy questions: which data the app collects, and what for.",
			Some(format!("{app}/distribution/privacy")),
			Vec::new(),
			"they are a legal declaration about the app's data, which the API can't make",
			"a submit succeeds",
		),
		step(
			"app_store",
			Who::Person,
			"Answer the age rating questions, in App Information.",
			Some(format!("{app}/distribution/info")),
			Vec::new(),
			"a content rating's answers are the account holder's declaration",
			"a submit succeeds",
		),
		step(
			"app_store",
			Who::Person,
			"Give App Review a contact, and a demo account if the app needs signing in, on the \
			 version's page.",
			Some(format!("{app}/distribution/ios/version/inflight")),
			Vec::new(),
			"the contact details and the account are the account holder's own",
			"a submit succeeds",
		),
		step(
			"app_store",
			Who::Person,
			"Decide to release it, then run `leptoskit publish app_store --submit`.",
			None,
			Vec::new(),
			"submitting sends it to App Review and then to users",
			"App Store Connect shows the version waiting for review",
		),
	]);
	steps
}

impl Session<'_> {
	/// One store's upload, and the steps left in it.
	fn publish(&self, store: &str) -> Result<(String, Vec<Step>)> {
		let (summary, mut steps) = match store {
			"chrome" => self.chrome()?,
			"edge" => self.edge()?,
			"firefox" => self.firefox()?,
			"app_store" => self.app_store()?,
			"macos" => self.macos()?,
			"play" => self.play()?,
			other => anyhow::bail!("`{other}` isn't a store leptoskit publish publishes to"),
		};
		if let Some(step) = icon_step(self.root, self.config, store) {
			steps.insert(0, step);
		}
		Ok((summary, steps))
	}
}

/// A step to give the app an icon, when `store` needs one it doesn't have.
fn icon_step(root: &Path, config: &LeptoskitConfig, store: &str) -> Option<Step> {
	let extension = EXTENSION_STORES.contains(&store);
	let has_icon =
		config.app.icon.is_some() || (extension && root.join("assets/icon-128.png").is_file());
	if has_icon {
		return None;
	}
	Some(step(
		store,
		Who::Agent,
		"Give the app an icon: set app.icon in leptoskit.toml to a square PNG or SVG, 1024×1024 \
		 or larger, and run this again.",
		None,
		vec![field(
			"What it makes",
			"the iOS app icon, Android's launcher icons, the macOS .icns, and the extension's 16 \
			 to 128 pixel icons",
		)],
		"stores refuse an app without its icon sizes, and leptoskit makes them all from one image",
		"app.icon is set",
	))
}

/// Asks before submitting, in a terminal; `--yes` and CI skip the question.
fn confirm(args: &PublishArgs, name: &str, version: &str, stores: &[&str]) -> Result<bool> {
	if args.yes || std::env::var_os("CI").is_some() || !std::io::stdin().is_terminal() {
		return Ok(true);
	}
	eprint!(
		"Submit {name} {version} to {} for review? Reviewed versions reach users. [y/N] ",
		stores.join(", ")
	);
	let mut answer = String::new();
	std::io::stdin().read_line(&mut answer)?;
	anyhow::ensure!(
		matches!(answer.trim(), "y" | "Y" | "yes"),
		"nothing was submitted"
	);
	Ok(true)
}

fn list_credentials(stores: &[&str], credentials: &Credentials) -> String {
	use std::fmt::Write as _;

	let mut out = format!("Credentials for {}:\n\n", stores.join(", "));
	for needed in NEEDED
		.iter()
		.filter(|needed| stores.contains(&needed.store))
	{
		let state = if credentials.get(needed.name).is_some() {
			"set"
		} else {
			"missing"
		};
		let _ = writeln!(out, "  {:<10} {:<42} {state}", needed.store, needed.name);
		if state == "missing" {
			let _ = writeln!(out, "             To make it: {}.", needed.how);
		}
	}
	out.push_str(
		"\nSet each as an environment variable, or put them in a TOML file outside the project \
		 and pass --credentials <file>. Values are never printed.\n",
	);
	out
}

fn step(
	store: &str,
	by: Who,
	what: &str,
	url: Option<String>,
	enter: Vec<Field>,
	why: &str,
	done_when: &str,
) -> Step {
	Step {
		store: store.to_owned(),
		by,
		what: what.to_owned(),
		url,
		enter,
		why: why.to_owned(),
		done_when: done_when.to_owned(),
	}
}

fn field(label: &str, value: impl Into<String>) -> Field {
	Field {
		label: label.to_owned(),
		value: value.into(),
	}
}

fn description(config: &LeptoskitConfig, store: &Store) -> String {
	store
		.description
		.clone()
		.or_else(|| {
			config
				.extension
				.as_ref()
				.and_then(|extension| extension.description.clone())
		})
		.unwrap_or_else(|| "(write it in store/en-US/description.md)".to_owned())
}

fn privacy_policy(store: &Store) -> String {
	store
		.listing
		.privacy_policy
		.clone()
		.unwrap_or_else(|| "(add privacy_policy to store/listing.toml)".to_owned())
}

/// What the Chrome Web Store's API can't set.
fn chrome_steps(
	config: &LeptoskitConfig,
	store: &Store,
	manifest: &serde_json::Value,
	submitted: bool,
) -> Vec<Step> {
	let settings = config.publish.chrome.as_ref().expect("configured");
	let item = format!(
		"https://chrome.google.com/webstore/devconsole/{}/{}",
		settings.publisher_id, settings.item_id
	);
	let mut justifications = vec![field(
		"Single purpose",
		config
			.extension
			.as_ref()
			.and_then(|extension| extension.description.clone())
			.unwrap_or_else(|| "(one sentence: what the extension is for)".to_owned()),
	)];
	for permission in manifest["permissions"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|value| value.as_str())
	{
		justifications.push(field(permission, store.justification(permission)));
	}
	if manifest.get("host_permissions").is_some()
		|| manifest.get("optional_host_permissions").is_some()
	{
		justifications.push(field(
			"Host permission",
			store.justification("host_permissions"),
		));
	}
	let mut steps = vec![
		step(
			"chrome",
			Who::Agent,
			"Fill in the store listing: the description, category, and language, a 128×128 icon, \
			 and at least one 1280×800 screenshot.",
			Some(format!("{item}/edit")),
			vec![
				field("Description", description(config, store)),
				field(
					"Category",
					store.listing.category.clone().unwrap_or_else(|| {
						"(choose one, and add category to store/listing.toml)".to_owned()
					}),
				),
				field(
					"Homepage",
					store.listing.homepage.clone().unwrap_or_default(),
				),
				field("Support", store.listing.support.clone().unwrap_or_default()),
			],
			"the Chrome Web Store API uploads packages but can't edit the listing",
			"a submit succeeds; the store refuses one while the listing is incomplete",
		),
		step(
			"chrome",
			Who::Agent,
			"Fill in the privacy practices: the single purpose, a justification for each \
			 permission, and the privacy policy's address.",
			Some(format!("{item}/edit/privacy")),
			justifications
				.into_iter()
				.chain([field("Privacy policy", privacy_policy(store))])
				.collect(),
			"the API has no privacy practices endpoint",
			"a submit succeeds",
		),
		step(
			"chrome",
			Who::Person,
			"Certify, on the same page, that the extension's data use follows the Developer \
			 Program Policies.",
			Some(format!("{item}/edit/privacy")),
			Vec::new(),
			"it is a legal declaration, which only the account holder can make",
			"a submit succeeds",
		),
	];
	if !submitted {
		steps.push(step(
			"chrome",
			Who::Person,
			"Decide to release it, then run `leptoskit publish chrome --submit`.",
			None,
			Vec::new(),
			"submitting sends it to review and then to users, which is the account holder's \
			 decision",
			"the store shows the version as pending review",
		));
	}
	steps
}

/// What Edge Add-ons' API can't set.
fn edge_steps(config: &LeptoskitConfig, store: &Store, submitted: bool) -> Vec<Step> {
	let settings = config.publish.edge.as_ref().expect("configured");
	let product = format!(
		"https://partner.microsoft.com/dashboard/microsoftedge/{}",
		settings.product_id
	);
	let mut steps = vec![step(
		"edge",
		Who::Agent,
		"Fill in the product's Properties (category, privacy policy, website, support) and its \
		 Store listing (description and screenshots).",
		Some(format!("{product}/properties")),
		vec![
			field("Description", description(config, store)),
			field("Privacy policy", privacy_policy(store)),
			field(
				"Website",
				store.listing.homepage.clone().unwrap_or_default(),
			),
			field("Support", store.listing.support.clone().unwrap_or_default()),
			field(
				"Category",
				store.listing.category.clone().unwrap_or_default(),
			),
		],
		"Edge Add-ons' Publish API uploads packages and submits them, but can't edit the listing",
		"a submit succeeds",
	)];
	if !submitted {
		steps.push(step(
			"edge",
			Who::Person,
			"Decide to release it, then run `leptoskit publish edge --submit`.",
			None,
			Vec::new(),
			"submitting sends it to certification and then to users",
			"Partner Center shows the submission in certification",
		));
	}
	steps
}

/// What addons.mozilla.org's API can't do.
fn firefox_steps(
	guid: &str,
	settings: &leptoskit_config::publish::FirefoxStore,
	store: &Store,
	submitted: bool,
	exists: bool,
	zip: &Path,
	source: &Path,
) -> Vec<Step> {
	let mut steps = Vec::new();
	if !exists {
		steps.push(step(
			"firefox",
			Who::Agent,
			"Submit the add-on's first version on the site, uploading the package and its source.",
			Some("https://addons.mozilla.org/developers/addon/submit/distribution".to_owned()),
			vec![
				field("Package", zip.display().to_string()),
				field("Source code", source.display().to_string()),
				field("Build instructions", "BUILD.md, inside the source archive"),
			],
			"the API publishes versions of an add-on that exists; creating one asks for a license \
			 and categories",
			"`leptoskit publish firefox --submit` creates versions instead",
		));
		steps.push(step(
			"firefox",
			Who::Person,
			"Choose the add-on's license on that page.",
			None,
			Vec::new(),
			"a license is a legal choice for the account holder",
			"the first submission is done",
		));
	}
	if settings.channel == leptoskit_config::publish::FirefoxChannel::Listed {
		steps.push(step(
			"firefox",
			Who::Agent,
			"Add the listing's screenshots and the privacy policy.",
			Some(format!(
				"https://addons.mozilla.org/developers/addon/{guid}/edit"
			)),
			std::iter::once(field("Privacy policy", privacy_policy(store)))
				.chain(missing(&[
					(
						"Summary",
						store.summary.is_some(),
						"write it in store/en-US/summary.md, 250 characters at most",
					),
					(
						"Description",
						store.description.is_some(),
						"write it in store/en-US/description.md",
					),
				]))
				.collect(),
			"leptoskit uploads the name, summary, and description with each upload, but not \
			 screenshots or the privacy policy",
			"the listing page shows them",
		));
	}
	if !submitted {
		steps.push(step(
			"firefox",
			Who::Person,
			"Decide to release it, then run `leptoskit publish firefox --submit`.",
			None,
			Vec::new(),
			"a listed version goes to review and then to users",
			"addons.mozilla.org shows the version awaiting review",
		));
	}
	steps
}

#[cfg(test)]
mod tests;
