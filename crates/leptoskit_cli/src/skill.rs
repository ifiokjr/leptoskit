//! `leptoskit skill`: the agent skill for this version of the CLI.
//!
//! The skill lives in `skills/leptoskit/` in the leptoskit repository, with
//! `files.txt` listing its files. The CLI fetches it from the GitHub tag for
//! its own version, so the skill always describes the commands this CLI
//! has. A CLI built from a checkout reads the checkout's copy instead.

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;

/// Where tagged files are fetched from.
const RAW: &str = "https://raw.githubusercontent.com/ifiokjr/leptoskit";
/// The skill's folder in the repository.
const FOLDER: &str = "skills/leptoskit";
/// The file an install leaves, naming the version and the files it wrote.
const MARKER: &str = ".leptoskit-skill";

/// `leptoskit skill`'s subcommands.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum SkillCommand {
	/// Installs the skill: into `.claude/skills/leptoskit` and
	/// `.agents/skills/leptoskit` here, `--global` for every project, or
	/// `--dir`.
	Install {
		/// Install into this folder instead.
		#[arg(long, conflicts_with = "global")]
		dir: Option<PathBuf>,
		/// Install into `~/.claude/skills/leptoskit`, for every project.
		#[arg(long)]
		global: bool,
		/// Replace a folder this command didn't write.
		#[arg(long)]
		force: bool,
	},
}

/// `leptoskit skill`'s arguments.
#[derive(Debug, Clone, clap::Args)]
pub struct SkillArgs {
	#[command(subcommand)]
	pub command: Option<SkillCommand>,
	/// The git revision to fetch the skill from, such as `main`; the tag for
	/// this CLI's version by default.
	#[arg(long = "ref", global = true)]
	pub git_ref: Option<String>,
}

/// The skill's files, as `(relative path, contents)`.
type Files = Vec<(String, String)>;

/// A checkout's copy, when this CLI was built from one.
fn checkout() -> Option<PathBuf> {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.join(FOLDER);
	dir.join("files.txt").is_file().then_some(dir)
}

fn listed(list: &str) -> Vec<String> {
	list.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
		.map(str::to_owned)
		.collect()
}

/// A listed name, checked to stay inside the directory the skill installs
/// into: relative, and normal components only, so a hostile `files.txt`
/// cannot write or remove files anywhere else on the machine.
fn inside(name: &str) -> Result<&Path> {
	let path = Path::new(name);
	anyhow::ensure!(
		!name.is_empty()
			&& path.is_relative()
			&& path
				.components()
				.all(|component| matches!(component, std::path::Component::Normal(_))),
		"{name:?} is not a file inside the skill"
	);
	Ok(path)
}

fn read_local(dir: &Path) -> Result<Files> {
	let list = std::fs::read_to_string(dir.join("files.txt"))?;
	listed(&list)
		.into_iter()
		.map(|name| {
			let contents = std::fs::read_to_string(dir.join(&name))
				.with_context(|| format!("could not read {name} from the skill"))?;
			// A Windows checkout has CRLF endings; GitHub serves LF.
			Ok((name, contents.replace("\r\n", "\n")))
		})
		.collect()
}

/// Fetches the skill at `git_ref` from `base`.
fn fetch(base: &str, git_ref: &str) -> Result<Files> {
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(Duration::from_secs(30)))
		.http_status_as_error(false)
		.build()
		.into();
	let get = |name: &str| -> Result<String> {
		let url = format!("{base}/{git_ref}/{FOLDER}/{name}");
		let mut response = agent.get(&url).call().with_context(|| {
			format!(
				"could not reach {url}; check your connection, then run `leptoskit skill` again"
			)
		})?;
		anyhow::ensure!(
			response.status() == 200,
			"{url} answered {}; pass --ref main for a CLI that isn't a release",
			response.status()
		);
		Ok(response.body_mut().read_to_string()?)
	};
	let list = get("files.txt")?;
	listed(&list)
		.into_iter()
		.map(|name| Ok((name.clone(), get(&name)?)))
		.collect()
}

fn load(git_ref: Option<&str>) -> Result<(Files, String)> {
	if let Some(git_ref) = git_ref {
		return Ok((fetch(RAW, git_ref)?, git_ref.to_owned()));
	}
	if let Some(dir) = checkout() {
		return Ok((read_local(&dir)?, format!("checkout {}", dir.display())));
	}
	let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
	Ok((fetch(RAW, &tag)?, tag))
}

/// The skill as one document: `SKILL.md`, then each reference under its
/// path.
fn printed(files: &Files) -> String {
	let mut out = String::new();
	for (name, contents) in files {
		if name != "SKILL.md" {
			let _ = write!(out, "\n\n<!-- {name} -->\n\n");
		}
		out.push_str(contents.trim_end());
		out.push('\n');
	}
	out
}

/// Writes the skill into `dir`, replacing only what an earlier install
/// wrote.
fn install(dir: &Path, files: &Files, source: &str, force: bool) -> Result<()> {
	let marker = dir.join(MARKER);
	if dir.exists() && !marker.is_file() && std::fs::read_dir(dir)?.next().is_some() {
		anyhow::ensure!(
			force,
			"{} has files leptoskit didn't write; pass --force to replace them",
			dir.display()
		);
	}
	if let Ok(previous) = std::fs::read_to_string(&marker) {
		for name in listed(&previous).into_iter().skip(1) {
			if !files.iter().any(|(file, _)| *file == name) {
				let _ = std::fs::remove_file(dir.join(inside(&name)?));
			}
		}
	}
	for (name, contents) in files {
		let path = dir.join(inside(name)?);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(&path, contents)
			.with_context(|| format!("could not write {}", path.display()))?;
	}
	let mut written = format!("# leptoskit skill from {source}\n");
	for (name, _) in files {
		written.push_str(name);
		written.push('\n');
	}
	std::fs::write(marker, written)?;
	Ok(())
}

/// `leptoskit skill`.
///
/// # Errors
///
/// When the skill can't be fetched or written.
pub fn run(args: &SkillArgs) -> Result<()> {
	let (files, source) = load(args.git_ref.as_deref())?;
	match &args.command {
		None => print!("{}", printed(&files)),
		Some(SkillCommand::Install { dir, global, force }) => {
			let dirs: Vec<PathBuf> = match (dir, global) {
				(Some(dir), _) => vec![dir.clone()],
				(None, true) => {
					let home = std::env::var_os("HOME")
						.or_else(|| std::env::var_os("USERPROFILE"))
						.context("no home folder to install into")?;
					vec![PathBuf::from(home).join(".claude/skills/leptoskit")]
				}
				(None, false) => {
					vec![
						PathBuf::from(".claude/skills/leptoskit"),
						PathBuf::from(".agents/skills/leptoskit"),
					]
				}
			};
			for dir in dirs {
				install(&dir, &files, &source, *force)?;
				println!(
					"installed the leptoskit skill ({source}) in {}",
					dir.display()
				);
			}
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn files() -> Files {
		vec![
			("SKILL.md".into(), "# skill\n".into()),
			("references/publishing.md".into(), "# publishing\n".into()),
		]
	}

	#[test]
	fn the_checkout_s_skill_lists_its_files() {
		let dir = checkout().expect("this test runs in the repository");
		let files = read_local(&dir).unwrap();
		assert_eq!(files[0].0, "SKILL.md");
		assert!(
			files
				.iter()
				.any(|(name, _)| name == "references/publishing.md")
		);
		let printed = printed(&files);
		assert!(printed.starts_with("---\nname: leptoskit"));
		assert!(printed.contains("<!-- references/publishing.md -->"));
	}

	#[test]
	fn installs_replace_only_what_they_wrote() {
		let root = tempfile::tempdir().unwrap();
		let dir = root.path().join("skill");
		install(&dir, &files(), "v1", false).unwrap();
		assert_eq!(
			std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
			"# skill\n"
		);
		// A newer skill without a reference removes the old one.
		install(&dir, &files()[..1].to_vec(), "v2", false).unwrap();
		assert!(!dir.join("references/publishing.md").exists());
		assert!(
			std::fs::read_to_string(dir.join(MARKER))
				.unwrap()
				.contains("v2")
		);

		let theirs = root.path().join("theirs");
		std::fs::create_dir_all(&theirs).unwrap();
		std::fs::write(theirs.join("notes.md"), "mine").unwrap();
		assert!(install(&theirs, &files(), "v1", false).is_err());
		install(&theirs, &files(), "v1", true).unwrap();
	}

	#[test]
	fn fetches_the_files_the_list_names() {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		listener.set_nonblocking(true).unwrap();
		std::thread::spawn(move || {
			let runtime = tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
				.unwrap();
			runtime.block_on(async move {
				let listener = tokio::net::TcpListener::from_std(listener).unwrap();
				let router = axum::Router::new()
					.route(
						"/v9.9.9/skills/leptoskit/files.txt",
						axum::routing::get(|| async { "SKILL.md\n# a comment\nreferences/a.md\n" }),
					)
					.route(
						"/v9.9.9/skills/leptoskit/SKILL.md",
						axum::routing::get(|| async { "skill" }),
					)
					.route(
						"/v9.9.9/skills/leptoskit/references/a.md",
						axum::routing::get(|| async { "a" }),
					);
				axum::serve(listener, router).await.unwrap();
			});
		});
		let base = format!("http://{address}");
		let files = fetch(&base, "v9.9.9").unwrap();
		assert_eq!(
			files,
			vec![
				("SKILL.md".into(), "skill".into()),
				("references/a.md".into(), "a".into())
			]
		);
		let error = fetch(&base, "v0.0.0").unwrap_err().to_string();
		assert!(error.contains("--ref main"), "{error}");
	}

	#[test]
	fn names_that_escape_the_folder_are_refused() {
		assert_eq!(
			inside("references/publishing.md").unwrap(),
			Path::new("references/publishing.md")
		);
		for escape in ["", "..", "../escape", "/absolute", "a/../../b"] {
			assert!(inside(escape).is_err(), "{escape:?} must not install");
		}
	}
}
