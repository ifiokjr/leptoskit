//! Applying the migrations `leptoskit db` generates.
//!
//! Migrations live in `migrations/`, one directory each, listed in order in
//! `migrations/registry.txt`. The server embeds them with
//! `leptoskit::db::migrations!("../migrations")` and applies the pending ones
//! at startup with `db.migrate(MIGRATIONS)`. Each runs in its own
//! transaction and is recorded, with a checksum of its SQL, in
//! `_leptoskit_migrations`.

use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;

use leptoskit_db_schema::FOREIGN_KEYS_OFF_DIRECTIVE;
use sha2::Digest;
use sha2::Sha256;

use crate::DbError;

/// The table that records applied migrations.
pub const MIGRATIONS_TABLE: &str = "_leptoskit_migrations";

/// The SQL that creates [`MIGRATIONS_TABLE`], valid in both dialects.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) const CREATE_MIGRATIONS_TABLE: &str =
	"CREATE TABLE IF NOT EXISTS \"_leptoskit_migrations\" (\"name\" TEXT NOT NULL PRIMARY KEY, \
	 \"checksum\" TEXT NOT NULL, \"applied_at\" TEXT NOT NULL)";

/// The table that records the schema a development database was pushed to,
/// without migrations, in `leptoskit dev`'s push mode: one row at most.
pub const PUSH_TABLE: &str = "_leptoskit_push";

/// The SQL that creates [`PUSH_TABLE`], valid in both dialects.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) const CREATE_PUSH_TABLE: &str =
	"CREATE TABLE IF NOT EXISTS \"_leptoskit_push\" (\"id\" INTEGER NOT NULL PRIMARY KEY CHECK \
	 (\"id\" = 1), \"schema\" TEXT NOT NULL, \"pushed_at\" TEXT NOT NULL)";

/// What applying a migration records.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) enum Record<'a> {
	/// That the migration was applied, in [`MIGRATIONS_TABLE`].
	Applied,
	/// The schema it leaves the database with, in [`PUSH_TABLE`], for a
	/// change pushed without a migration.
	Pushed(&'a str),
	/// That the migration was applied, without running it, because pushes
	/// already made its changes; the pushes are forgotten.
	Adopted,
	/// Runs a reverse migration and removes its history entry.
	Reverted,
}

/// One migration: its name, its SQL, and a checksum of that SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
	/// The migration's directory name, such as `20261102T1200_add_posts`.
	pub name: Cow<'static, str>,
	/// The contents of its `migration.sql`.
	pub sql: Cow<'static, str>,
	/// The SHA-256 of `sql`, in hex.
	pub checksum: Cow<'static, str>,
	/// Reviewed reverse SQL, when this migration supports rollback.
	pub down: Option<Cow<'static, str>>,
}

impl Migration {
	/// A migration embedded at compile time by `migrations!`.
	#[doc(hidden)]
	pub const fn embedded(name: &'static str, sql: &'static str, checksum: &'static str) -> Self {
		Self {
			name: Cow::Borrowed(name),
			sql: Cow::Borrowed(sql),
			checksum: Cow::Borrowed(checksum),
			down: None,
		}
	}

	/// A migration from its name and SQL, with the checksum computed.
	pub fn new(name: impl Into<String>, sql: impl Into<String>) -> Self {
		let sql = sql.into();
		Self {
			name: Cow::Owned(name.into()),
			checksum: Cow::Owned(checksum(&sql)),
			sql: Cow::Owned(sql),
			down: None,
		}
	}

	/// Attaches reverse SQL. A generated irreversible marker must be resolved
	/// by the author before this SQL can be used for rollback.
	#[must_use]
	pub fn with_down(mut self, sql: impl Into<String>) -> Self {
		self.down = Some(Cow::Owned(sql.into()));
		self
	}

	/// Embeds a reverse migration alongside its forward SQL.
	#[doc(hidden)]
	pub const fn embedded_reversible(
		name: &'static str,
		sql: &'static str,
		checksum: &'static str,
		down: &'static str,
	) -> Self {
		Self {
			name: Cow::Borrowed(name),
			sql: Cow::Borrowed(sql),
			checksum: Cow::Borrowed(checksum),
			down: Some(Cow::Borrowed(down)),
		}
	}

	/// Whether the migration asks for SQLite's foreign key enforcement to be
	/// off while it runs, because it rebuilds tables.
	pub fn foreign_keys_off(&self) -> bool {
		self.sql
			.lines()
			.any(|line| line.trim() == FOREIGN_KEYS_OFF_DIRECTIVE)
	}
}

/// The SHA-256 of a migration's SQL, in hex, as `migrations!` and
/// [`load`] compute it.
pub fn checksum(sql: &str) -> String {
	Sha256::digest(sql.as_bytes())
		.iter()
		.fold(String::with_capacity(64), |mut hex, byte| {
			let _ = write!(hex, "{byte:02x}");
			hex
		})
}

/// Reads `registry.txt` and every migration it lists from `directory`, at run
/// time. The server embeds them with `migrations!` instead; `leptoskit db`
/// uses this.
pub fn load(directory: &Path) -> Result<Vec<Migration>, MigrationError> {
	let registry_path = directory.join("registry.txt");
	let registry = read(&registry_path)?;

	registry_names(&registry)
		.map(|name| {
			let sql = read(&directory.join(name).join("migration.sql"))?;
			let mut migration = Migration::new(name, sql);
			let path = directory.join(name).join("down.sql");
			match std::fs::read_to_string(&path) {
				Ok(down) => migration = migration.with_down(down),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(source) => return Err(MigrationError::Read { path, source }),
			}
			Ok(migration)
		})
		.collect()
}

/// The migration names in a `registry.txt`: one per line, ignoring blank
/// lines and `#` comments.
pub fn registry_names(registry: &str) -> impl Iterator<Item = &str> {
	registry
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// A release marker from a `registry.txt`: `# release <version>
/// [(<source>)] snapshot <sha256>`, recording that every migration above it
/// shipped, so it can no longer change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release<'a> {
	/// The release's version, such as `1.4.0`.
	pub version: &'a str,
	/// Where the release came from, such as a monochange release id.
	pub source: Option<&'a str>,
	/// The SHA-256 of the `schema.json` the release ends at, which pins the
	/// migrations above the marker.
	pub snapshot: &'a str,
}

/// Parses one release marker line, trimmed. Any line that starts with
/// `# release ` but fails to parse is a marker with a typo in it, which the
/// caller should refuse rather than silently unfreeze a release.
pub fn parse_release(line: &str) -> Option<Release<'_>> {
	let rest = line.strip_prefix("# release ")?;
	let (rest, snapshot) = rest.split_once(" snapshot ")?;
	let (version, source) = match rest.split_once(" (") {
		Some((version, source)) if source.ends_with(')') => {
			(version, Some(&source[..source.len() - 1]))
		}
		_ => (rest, None),
	};
	(!version.is_empty() && !snapshot.is_empty()).then_some(Release {
		version,
		source,
		snapshot,
	})
}

/// The release markers in a `registry.txt`, in the order they appear.
pub fn registry_releases(registry: &str) -> impl Iterator<Item = Release<'_>> {
	registry
		.lines()
		.map(str::trim)
		.filter(|line| line.starts_with("# release "))
		.filter_map(parse_release)
}

fn read(path: &Path) -> Result<String, MigrationError> {
	std::fs::read_to_string(path).map_err(|source| {
		MigrationError::Read {
			path: path.to_path_buf(),
			source,
		}
	})
}

/// The migrations still to apply, after checking the applied ones against
/// `migrations`: an applied migration whose SQL changed, or that this build
/// doesn't know, stops everything.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn pending<'a>(
	applied: &[(String, String)],
	migrations: &'a [Migration],
) -> Result<Vec<&'a Migration>, MigrationError> {
	for (name, checksum) in applied {
		let Some(migration) = migrations.iter().find(|migration| migration.name == *name) else {
			return Err(MigrationError::Unknown { name: name.clone() });
		};

		if migration.checksum != *checksum {
			return Err(MigrationError::Changed { name: name.clone() });
		}
	}

	Ok(migrations
		.iter()
		.filter(|migration| !applied.iter().any(|(name, _)| migration.name == *name))
		.collect())
}

/// Preflights the entire rollback before any reverse SQL executes. Only an
/// applied registry prefix can be rolled back, newest first.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn reversing(
	applied: &[(String, String)],
	migrations: &[Migration],
	steps: usize,
) -> Result<Vec<Migration>, MigrationError> {
	let _ = pending(applied, migrations)?;
	if steps == 0
		|| steps > applied.len()
		|| migrations
			.iter()
			.take(applied.len())
			.any(|m| !applied.iter().any(|(n, _)| n == &m.name))
	{
		return Err(MigrationError::RollbackHistory);
	}
	migrations[..applied.len()]
		.iter()
		.rev()
		.take(steps)
		.map(|migration| {
			let down = migration
				.down
				.as_deref()
				.filter(|sql| {
					!sql.trim().is_empty()
						&& !sql
							.lines()
							.any(|line| line.trim() == "-- leptoskit: irreversible")
				})
				.ok_or_else(|| {
					MigrationError::Irreversible {
						name: migration.name.to_string(),
					}
				})?;
			Ok(Migration::new(migration.name.to_string(), down))
		})
		.collect()
}

/// Why migrating stopped.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
	/// A reverse migration is missing or needs manual implementation.
	#[error("migration `{name}` has no reviewed down.sql; rollback stopped before making changes")]
	Irreversible {
		/// The migration.
		name: String,
	},
	/// Rollback needs a positive count and an intact applied prefix.
	#[error(
		"rollback needs an applied registry prefix and a positive step count no larger than its \
		 length"
	)]
	RollbackHistory,
	/// A migration file couldn't be read.
	#[error("couldn't read {}: {source}", path.display())]
	Read {
		/// The file.
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	/// An applied migration's SQL differs from this build's copy.
	#[error(
		"migration `{name}` was applied, and its SQL has changed since. Applied migrations are \
		 history: undo the edit and make the change in a new migration"
	)]
	Changed {
		/// The migration.
		name: String,
	},
	/// The database has a migration this build doesn't know, such as after
	/// deploying older code.
	#[error("migration `{name}` was applied to this database, but this build doesn't have it")]
	Unknown {
		/// The migration.
		name: String,
	},
	/// A migration left foreign keys pointing at missing rows.
	#[error("migration `{name}` left {violations} foreign key violation(s), so it was rolled back")]
	ForeignKeys {
		/// The migration.
		name: String,
		/// How many rows violate a foreign key.
		violations: i64,
	},
	/// A migration's SQL failed; it was rolled back.
	#[error("migration `{name}` failed and was rolled back")]
	Failed {
		/// The migration.
		name: String,
		#[source]
		source: DbError,
	},
	/// The database holds changes pushed without a migration, and a
	/// migration it hasn't had would apply on top of them.
	#[error(
		"the database has changes pushed in development that no migration records, so migration \
		 `{name}` can't apply on top of them; run `leptoskit db migration create` to turn the \
		 pushes into a migration, or `leptoskit db reset` to start again"
	)]
	Pushed {
		/// The first migration that would have applied.
		name: String,
	},
	/// Adopting a migration needs pushes to adopt.
	#[error("migration `{name}` can't be adopted: the database has no pushed changes")]
	NothingPushed {
		/// The migration.
		name: String,
	},
	/// The database couldn't be read or locked before migrating.
	#[error("couldn't prepare the database for migrations")]
	Database(#[from] DbError),
}
