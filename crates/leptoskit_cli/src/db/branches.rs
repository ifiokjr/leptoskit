//! Branch databases: on the development Postgres, each git branch gets its
//! own database, copied from the main branch's the first time it's used, so
//! switching branches never mixes one branch's tables or rows with
//! another's.
//!
//! The main branch keeps the app's database, `app`; another branch uses
//! `app__<branch>`, created with `CREATE DATABASE … TEMPLATE app` and
//! labelled with its branch in a comment, which is how `leptoskit db branch
//! prune` later finds the ones whose branch is gone.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use leptoskit_db::Read;
use leptoskit_db::Db;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db_schema::quote_identifier;

use super::Project;

/// Postgres names are at most 63 bytes.
const MAX_NAME: usize = 63;

/// What a branch database's comment starts with, before the branch's name.
const LABEL: &str = "leptoskit branch: ";

/// Where the project is in git: the branch it's on, `None` when detached,
/// and its main branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branches {
	pub current: Option<String>,
	pub main: String,
}

/// The project's branches, or `None` outside a git repository.
pub(crate) fn branches(root: &Path) -> Option<Branches> {
	let current = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
	let main = git(
		root,
		&[
			"symbolic-ref",
			"--quiet",
			"--short",
			"refs/remotes/origin/HEAD",
		],
	)
	.and_then(|remote| remote.split_once('/').map(|(_, branch)| branch.to_owned()))
	.or_else(|| {
		["main", "master"]
			.into_iter()
			.find(|branch| {
				git(
					root,
					&[
						"show-ref",
						"--verify",
						"--quiet",
						&format!("refs/heads/{branch}"),
					],
				)
				.is_some()
			})
			.map(ToOwned::to_owned)
	})
	.unwrap_or_else(|| current.clone());

	Some(Branches {
		current: (current != "HEAD").then_some(current),
		main,
	})
}

/// The file git rewrites when the project switches branch.
pub(crate) fn head_file(root: &Path) -> Option<std::path::PathBuf> {
	let path = std::path::PathBuf::from(git(root, &["rev-parse", "--git-path", "HEAD"])?);
	Some(if path.is_absolute() {
		path
	} else {
		root.join(path)
	})
}

/// What `git` prints, trimmed, when it succeeds.
fn git(root: &Path, args: &[&str]) -> Option<String> {
	let output = crate::git::command(root).args(args).output().ok()?;
	output
		.status
		.success()
		.then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The database `branch` uses: `main_database` for the main branch, and
/// `<main_database>__<branch>` otherwise, in lowercase letters, digits,
/// and `_`, and shortened with a hash of the branch's name to fit.
pub(crate) fn database_for(main_database: &str, branches: &Branches) -> String {
	let Some(branch) = branches
		.current
		.as_deref()
		.filter(|branch| *branch != branches.main)
	else {
		return main_database.to_owned();
	};
	let mut slug = String::new();

	for character in branch.chars() {
		if character.is_ascii_alphanumeric() {
			slug.push(character.to_ascii_lowercase());
		} else if !slug.ends_with('_') {
			slug.push('_');
		}
	}

	let name = format!("{main_database}__{}", slug.trim_matches('_'));

	if name.len() <= MAX_NAME {
		return name;
	}

	let hash = &leptoskit_db::checksum(branch)[..8];
	let mut short = name[..MAX_NAME - hash.len() - 1].to_owned();

	while !short.is_char_boundary(short.len()) {
		short.pop();
	}

	format!("{short}_{hash}")
}

/// The database the project uses on the development server at `url`, which
/// names its `postgres` database: the branch's, copied from the main
/// branch's when it doesn't exist yet. Returns its name.
pub(crate) fn ensure(project: &Project, url: &str) -> Result<String> {
	let main = project.database_name();
	let branches = project
		.config
		.branch_databases
		.then(|| branches(&project.root))
		.flatten();
	let Some(branches) = branches else {
		ensure_database(url, &main, None)?;
		return Ok(main);
	};

	if branches.current.is_none() {
		println!("[db] HEAD is detached, so the main branch's database `{main}` is used");
	}

	let name = database_for(&main, &branches);
	ensure_database(url, &main, None)?;

	if name != main {
		let branch = branches.current.as_deref().unwrap_or_default();

		if ensure_database(url, &name, Some((&main, branch)))? {
			println!("[db] branch `{branch}` has its own database, `{name}`, copied from `{main}`");
		}
	}

	Ok(name)
}

/// Creates `database` unless it exists: empty, or as a copy of `template`
/// labelled with its branch. Returns whether it created it.
pub(crate) fn ensure_database(
	url: &str,
	database: &str,
	template: Option<(&str, &str)>,
) -> Result<bool> {
	super::runtime()?.block_on(async {
		let db = Db::connect(url).await?;

		if exists(&db, database).await? {
			return Ok(false);
		}

		let name = quote_identifier(database);
		let Some((template, branch)) = template else {
			db.run_script(&format!("CREATE DATABASE {name}")).await?;
			return Ok(true);
		};

		if let Err(error) = db
			.run_script(&format!(
				"CREATE DATABASE {name} TEMPLATE {}",
				quote_identifier(template)
			))
			.await
		{
			let message = std::iter::successors(
				Some(&error as &(dyn std::error::Error + 'static)),
				|error| error.source(),
			)
			.map(ToString::to_string)
			.collect::<Vec<_>>()
			.join(": ");

			if message.contains("being accessed by other users") {
				bail!(
					"couldn't copy `{template}` for branch `{branch}`: something is connected to \
					 it, such as an app server or psql; stop it and try again"
				);
			}

			return Err(error.into());
		}

		db.run_script(&format!(
			"COMMENT ON DATABASE {name} IS {}",
			literal(&format!("{LABEL}{branch}"))
		))
		.await?;
		anyhow::Ok(true)
	})
}

async fn exists(db: &Db, database: &str) -> Result<bool> {
	let found = db
		.fetch(Statement {
			sql: "SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1".to_owned(),
			args: vec![Value::Text(database.to_owned())],
		})
		.await?;
	Ok(!found.is_empty())
}

/// A SQL string literal.
fn literal(text: &str) -> String {
	format!("'{}'", text.replace('\'', "''"))
}

/// One branch database on the development server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchDatabase {
	pub name: String,
	pub branch: String,
}

/// The branch databases of the project on the server at `url`.
pub(crate) fn list(project: &Project, url: &str) -> Result<Vec<BranchDatabase>> {
	let prefix = format!("{}__", project.database_name());
	super::runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		let rows = db
			.fetch(Statement {
				sql: "SELECT datname, shobj_description(oid, 'pg_database') FROM \
				      pg_catalog.pg_database WHERE starts_with(datname, $1) ORDER BY datname"
					.to_owned(),
				args: vec![Value::Text(prefix)],
			})
			.await?;

		Ok(rows
			.into_iter()
			.filter_map(|row| {
				let mut values = row.into_iter();
				let (Some(Value::Text(name)), Some(Value::Text(comment))) =
					(values.next(), values.next())
				else {
					return None;
				};
				let branch = comment.strip_prefix(LABEL)?.to_owned();
				Some(BranchDatabase { name, branch })
			})
			.collect())
	})
}

/// Why a branch database can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gone {
	/// Its branch no longer exists.
	Deleted,
	/// Its branch is merged into the main branch.
	Merged,
}

/// Which of `databases` belong to branches that are gone or merged into
/// the main branch. The branch checked out now, and the main branch, always
/// stay.
pub(crate) fn prunable(
	root: &Path,
	databases: &[BranchDatabase],
) -> Result<Vec<(BranchDatabase, Gone)>> {
	let branches = branches(root).context("branch databases need a git repository")?;
	let merged = git(
		root,
		&[
			"branch",
			"--format=%(refname:short)",
			"--merged",
			&branches.main,
		],
	)
	.unwrap_or_default();
	let merged: Vec<&str> = merged.lines().map(str::trim).collect();
	let mut gone = Vec::new();

	for database in databases {
		if Some(&database.branch) == branches.current.as_ref() || database.branch == branches.main {
			continue;
		}

		let exists = git(
			root,
			&[
				"show-ref",
				"--verify",
				"--quiet",
				&format!("refs/heads/{}", database.branch),
			],
		)
		.is_some();

		if !exists {
			gone.push((database.clone(), Gone::Deleted));
		} else if merged.contains(&database.branch.as_str()) {
			gone.push((database.clone(), Gone::Merged));
		}
	}

	Ok(gone)
}

/// Drops `database` from the server at `url`, closing whatever is connected
/// to it: it's a development copy.
pub(crate) fn drop_database(url: &str, database: &str) -> Result<()> {
	super::runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		db.run_script(&format!(
			"DROP DATABASE IF EXISTS {} WITH (FORCE)",
			quote_identifier(database)
		))
		.await?;
		anyhow::Ok(())
	})
}

#[cfg(test)]
mod tests {
	use leptoskit_config::DbConfig;
	use leptoskit_config::DbDevServer;
	use leptoskit_config::DbDialect;

	use super::*;

	/// Runs git in a throwaway repository, away from the user's own
	/// configuration.
	fn run(root: &Path, args: &[&str]) {
		let status = crate::git::command(root)
			.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
			.args(args)
			.env("GIT_CONFIG_GLOBAL", root.join("no-global-config"))
			.env("GIT_CONFIG_NOSYSTEM", "1")
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.unwrap();
		assert!(status.success(), "git {args:?}");
	}

	fn repository() -> tempfile::TempDir {
		let root = tempfile::tempdir().unwrap();
		run(root.path(), &["init", "--quiet", "--initial-branch=main"]);
		run(
			root.path(),
			&["commit", "--quiet", "--allow-empty", "-m", "first"],
		);
		root
	}

	fn on(current: Option<&str>) -> Branches {
		Branches {
			current: current.map(ToOwned::to_owned),
			main: "main".into(),
		}
	}

	#[test]
	fn each_branch_names_its_own_database() {
		assert_eq!(database_for("app", &on(Some("main"))), "app");
		assert_eq!(database_for("app", &on(None)), "app");
		assert_eq!(
			database_for("app", &on(Some("feat/Push-Mode"))),
			"app__feat_push_mode"
		);
		assert_eq!(database_for("app", &on(Some("--x--"))), "app__x");

		let long = "feat/".to_owned() + &"very-long-branch-name-".repeat(5);
		let name = database_for("my_app", &on(Some(&long)));
		assert!(name.len() <= MAX_NAME, "{name}");
		assert!(name.starts_with("my_app__feat_very_long"), "{name}");
		let other = database_for("my_app", &on(Some(&(long.clone() + "2"))));
		assert_ne!(name, other, "a hash keeps long names apart");
	}

	#[test]
	fn git_says_which_branch_the_project_is_on() {
		let outside = tempfile::tempdir().unwrap();
		assert_eq!(branches(outside.path()), None);

		let root = repository();
		assert_eq!(branches(root.path()), Some(on(Some("main"))));
		assert!(head_file(root.path()).unwrap().ends_with("HEAD"));

		run(root.path(), &["checkout", "--quiet", "-b", "feature"]);
		assert_eq!(branches(root.path()), Some(on(Some("feature"))));
		run(root.path(), &["checkout", "--quiet", "--detach"]);
		assert_eq!(branches(root.path()), Some(on(None)));
	}

	#[test]
	fn branch_databases_are_copied_listed_and_pruned() {
		let Some(binaries) = super::super::dev_server::test_binaries() else {
			return;
		};
		let root = repository();
		let project = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Postgres,
				dev_server: DbDevServer::Embedded,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "my-app".into(),
		};
		let server = super::super::dev_server::start(
			&project,
			super::super::dev_server::Source::Embedded,
			Some(&binaries),
		)
		.unwrap();
		assert!(server.url.contains("/my_app?"), "{}", server.url);
		let postgres = super::super::dev_server::with_database(&server.url, "postgres").unwrap();
		let script = |database: &str, sql: &str| {
			let url = super::super::dev_server::with_database(&server.url, database).unwrap();
			super::super::runtime()
				.unwrap()
				.block_on(async {
					let db = Db::connect(&url).await?;
					db.run_script(sql).await
				})
				.unwrap();
		};
		script(
			"my_app",
			"CREATE TABLE notes (body text); INSERT INTO notes VALUES ('main')",
		);

		// A new branch copies the main branch's database, rows and all.
		run(root.path(), &["checkout", "--quiet", "-b", "feat/notes"]);
		assert_eq!(ensure(&project, &postgres).unwrap(), "my_app__feat_notes");
		script("my_app__feat_notes", "INSERT INTO notes VALUES ('branch')");
		assert_eq!(ensure(&project, &postgres).unwrap(), "my_app__feat_notes");
		let count = |database: &str| -> i64 {
			let url = super::super::dev_server::with_database(&server.url, database).unwrap();
			super::super::runtime()
				.unwrap()
				.block_on(async {
					let db = Db::connect(&url).await?;
					let rows = db
						.fetch(Statement {
							sql: "SELECT count(*) FROM notes".into(),
							args: Vec::new(),
						})
						.await?;
					anyhow::Ok(match rows[0][0] {
						Value::Int(count) => count,
						_ => -1,
					})
				})
				.unwrap()
		};
		assert_eq!((count("my_app"), count("my_app__feat_notes")), (1, 2));

		// Off, every branch shares the app's database.
		let mut shared = Project {
			root: project.root.clone(),
			config: project.config.clone(),
			package: project.package.clone(),
			name: project.name.clone(),
		};
		shared.config.branch_databases = false;
		assert_eq!(ensure(&shared, &postgres).unwrap(), "my_app");

		// A copy fails clearly while something holds the main branch's open.
		run(root.path(), &["checkout", "--quiet", "-b", "feat/busy"]);
		let main_url = super::super::dev_server::with_database(&server.url, "my_app").unwrap();
		let runtime = super::super::runtime().unwrap();
		let holder = runtime.block_on(async {
			let db = Db::connect(&main_url).await.unwrap();
			db.run_script("SELECT 1").await.unwrap();
			db
		});
		let busy = ensure(&project, &postgres).unwrap_err();
		assert!(
			busy.to_string().contains("something is connected"),
			"{busy}"
		);
		drop(holder);
		drop(runtime);
		assert_eq!(ensure(&project, &postgres).unwrap(), "my_app__feat_busy");

		let listed = list(&project, &postgres).unwrap();
		let branches_listed: Vec<&str> = listed
			.iter()
			.map(|database| database.branch.as_str())
			.collect();
		assert_eq!(branches_listed, ["feat/busy", "feat/notes"]);

		// The checked-out branch stays; a merged one and a deleted one go.
		run(root.path(), &["checkout", "--quiet", "main"]);
		run(root.path(), &["branch", "--quiet", "-D", "feat/busy"]);
		let mut gone = prunable(root.path(), &listed).unwrap();
		gone.sort_by(|left, right| left.0.name.cmp(&right.0.name));
		assert_eq!(
			gone.iter()
				.map(|(database, why)| (database.name.as_str(), *why))
				.collect::<Vec<_>>(),
			[
				("my_app__feat_busy", Gone::Deleted),
				("my_app__feat_notes", Gone::Merged),
			]
		);
		run(root.path(), &["checkout", "--quiet", "feat/notes"]);
		run(
			root.path(),
			&["commit", "--quiet", "--allow-empty", "-m", "unmerged"],
		);
		let gone = prunable(root.path(), &listed).unwrap();
		assert_eq!(gone.len(), 1, "the branch checked out now stays: {gone:?}");
		run(root.path(), &["checkout", "--quiet", "main"]);
		assert_eq!(
			prunable(root.path(), &listed).unwrap().len(),
			1,
			"unmerged work stays"
		);

		for (database, _) in &gone {
			drop_database(&postgres, &database.name).unwrap();
		}
		assert_eq!(list(&project, &postgres).unwrap().len(), 1);
		assert!(prunable(tempfile::tempdir().unwrap().path(), &listed).is_err());
	}
}
