//! `leptoskit publish`: from source to each store's review queue.
//!
//! Publishing has two steps on every store. **Upload** puts a build where
//! the store keeps it as a draft; `--submit` also sends it for review or
//! release, which reaches users, so nothing does without it. `--dry-run`
//! builds and checks everything and contacts no store. Every run ends with
//! the steps the stores' APIs can't take, which a person, or an agent with
//! `leptoskit skill`, finishes in each dashboard.

pub mod chrome;
pub mod edge;
pub mod firefox;
pub mod http;
pub mod listing;
pub mod secrets;
pub mod steps;

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
	/// The stores: `chrome`, `edge`, and `firefox`; every store `[publish]`
	/// configures when omitted. `credentials` lists the credentials each
	/// configured store needs instead.
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
	/// A TOML file of credentials, by their environment variables' names,
	/// outside the project.
	#[arg(long)]
	pub credentials: Option<PathBuf>,
}

/// Where the report and the store-ready files go.
const OUTPUT: &str = ".leptoskit/publish";

/// A credential a store needs.
struct Needed {
	store: &'static str,
	name: &'static str,
	how: &'static str,
}

const NEEDED: [Needed; 5] = [
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
	let configured = config.publish.stores();
	anyhow::ensure!(
		!configured.is_empty(),
		"no store is configured: add [publish.chrome], [publish.edge], or [publish.firefox] to \
		 leptoskit.toml"
	);
	if args.stores.iter().any(|store| store == "credentials") {
		list_credentials(&configured, &credentials);
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
	let built = crate::extension::build(true, None)?;
	let manifest: serde_json::Value = {
		let chromium = built
			.packages
			.iter()
			.find(|package| package.browser == leptoskit_config::Browser::Chromium)
			.or_else(|| built.packages.first())
			.context("the build made no package")?;
		serde_json::from_slice(&std::fs::read(chromium.dir.join("manifest.json"))?)?
	};
	let version = manifest["version"].as_str().unwrap_or_default().to_owned();
	let name = manifest["name"].as_str().unwrap_or_default().to_owned();
	let submit = args.submit && !args.dry_run && confirm(args, &name, &version, &stores)?;
	let http = http::Http::default();
	let session = Session {
		args,
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		built: &built,
		manifest: &manifest,
		name,
		version,
		submit,
	};
	let mut report = Report::default();
	for store_name in &stores {
		let (summary, steps) = match *store_name {
			"chrome" => session.chrome()?,
			"edge" => session.edge()?,
			"firefox" => session.firefox()?,
			other => anyhow::bail!("`{other}` isn't a store leptoskit publish publishes to"),
		};
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

/// Everything one run's stores share.
struct Session<'a> {
	args: &'a PublishArgs,
	config: &'a LeptoskitConfig,
	credentials: &'a Credentials,
	store: &'a Store,
	http: &'a http::Http,
	built: &'a crate::extension::Built,
	manifest: &'a serde_json::Value,
	name: String,
	version: String,
	submit: bool,
}

impl Session<'_> {
	/// A browser family's zip.
	fn package(&self, browser: leptoskit_config::Browser) -> Result<PathBuf> {
		self.built
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
			chrome_steps(self.config, self.store, self.manifest, self.submit),
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
		let zip = self.package(leptoskit_config::Browser::Firefox)?;
		let source = self
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
			if self.submit {
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
			}
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

fn list_credentials(stores: &[&str], credentials: &Credentials) {
	println!("Credentials for {}:\n", stores.join(", "));
	for needed in NEEDED
		.iter()
		.filter(|needed| stores.contains(&needed.store))
	{
		let state = if credentials.get(needed.name).is_some() {
			"set"
		} else {
			"missing"
		};
		println!("  {:<8} {:<42} {state}", needed.store, needed.name);
		if state == "missing" {
			println!("           To make it: {}.", needed.how);
		}
	}
	println!(
		"\nSet each as an environment variable, or put them in a TOML file outside the project \
		 and pass --credentials <file>. Values are never printed."
	);
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
			"Fill in the listing: the description, screenshots, and privacy policy.",
			Some(format!(
				"https://addons.mozilla.org/developers/addon/{guid}/edit"
			)),
			vec![
				field(
					"Description",
					store
						.description
						.clone()
						.unwrap_or_else(|| "(write it in store/en-US/description.md)".to_owned()),
				),
				field("Privacy policy", privacy_policy(store)),
			],
			"leptoskit doesn't upload listing text and images yet",
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
