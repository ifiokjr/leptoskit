//! Push mode: the development database follows the tables without
//! migrations, and `leptoskit db migration create` squashes everything
//! pushed into one migration.
//!
//! A push is the change from what the database holds to what the tables
//! declare, applied in a transaction and recorded, with the schema it
//! leaves, in `_leptoskit_push`. Pushes that would delete data need
//! `--allow-destructive`. While a database has pushes, it refuses
//! migrations it hasn't had, so a teammate's migration never lands on top
//! of half-finished changes; creating a migration adopts the pushes.

use std::fmt::Write as _;

use anyhow::Context;
use anyhow::Result;
use leptoskit_db::Db;
use leptoskit_db::Migration;
use leptoskit_db_schema::Plan;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Risk;
use leptoskit_db_schema::Schema;

use super::History;
use super::Project;
use crate::cli::invoked_name;

/// What a push did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pushed {
	/// Nothing: the database already has the tables.
	Unchanged,
	/// Applied these steps.
	Applied(Vec<String>),
	/// Nothing, because these steps delete data and that wasn't allowed.
	Refused(Vec<String>),
}

impl Pushed {
	/// What to tell the developer.
	pub(crate) fn message(&self) -> Option<String> {
		let mut message = String::new();

		match self {
			Self::Unchanged => return None,
			Self::Applied(steps) => {
				let _ = writeln!(message, "[db] pushed to the development database:");
				write_steps(&mut message, steps);
			}
			Self::Refused(steps) => {
				let _ = writeln!(
					message,
					"[db] the tables changed in a way that deletes data, so nothing was pushed:"
				);
				write_steps(&mut message, steps);
				let _ = writeln!(
					message,
					"[db] if that's intended, run `{} db push --allow-destructive`; if a column \
					 or table was renamed, mark it with #[db(renamed_from = \"...\")]",
					invoked_name()
				);
			}
		}

		Some(message)
	}
}

fn write_steps(message: &mut String, steps: &[String]) {
	for step in steps {
		let _ = writeln!(message, "  {step}");
	}
}

/// The schema the database holds: what pushes left it with, or the latest
/// migration's.
async fn holds(db: &Db, history: &History) -> Result<Schema> {
	match db.pushed_schema().await? {
		Some(json) => {
			Schema::from_json(&json).context("the development database's pushed schema is invalid")
		}
		None => Ok(history.latest.clone()),
	}
}

/// The steps of `plan`, with why the ones that delete data do.
fn described(plan: &Plan) -> Vec<String> {
	plan.steps
		.iter()
		.map(|step| {
			match step.risk() {
				Risk::LosesData(reason) => format!("{step}: {reason}"),
				_ => step.to_string(),
			}
		})
		.collect()
}

/// Brings the database at `url` to `target` without a migration: first
/// the migrations it hasn't had, then the difference.
pub(crate) fn push(
	project: &Project,
	url: &str,
	target: &Schema,
	allow_destructive: bool,
) -> Result<Pushed> {
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	let migrations = if history.names.is_empty() {
		Vec::new()
	} else {
		leptoskit_db::load_migrations(&project.migrations_dir())?
	};

	super::runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		db.migrate(&migrations).await?;
		push_to(&db, &history, target, allow_destructive).await
	})
}

async fn push_to(
	db: &Db,
	history: &History,
	target: &Schema,
	allow_destructive: bool,
) -> Result<Pushed> {
	let target = target.without_rename_hints();
	let current = holds(db, history).await?;
	let plan = leptoskit_db_schema::diff(&current, &target, &mut RequireHints)?;

	if plan.is_empty() {
		return Ok(Pushed::Unchanged);
	}

	let steps = described(&plan);

	if plan.destructive_steps().next().is_some() && !allow_destructive {
		return Ok(Pushed::Refused(steps));
	}

	let migration = leptoskit_db_schema::render(&plan, &current, &target);
	db.push(&Migration::new("push", migration.sql()), &target.to_json())
		.await?;

	// Back where the latest migration left it: nothing is pushed any more.
	if target == history.latest {
		db.run_script(&format!(
			"DELETE FROM {}",
			leptoskit_db_schema::quote_identifier(leptoskit_db::PUSH_TABLE)
		))
		.await?;
	}

	Ok(Pushed::Applied(steps))
}

/// A development database with pushes, about to have them squashed into a
/// migration.
pub(crate) struct Squash {
	db: Db,
	/// What the database still needs to match the tables.
	rest: Option<Migration>,
	runtime: tokio::runtime::Runtime,
}

/// Prepares to squash the pushes on the database at `url`, when it has
/// any, into a migration to `target`. Fails, before anything is written,
/// when catching the database up would delete data without
/// `allow_destructive`.
pub(crate) fn prepare_squash(
	project: &Project,
	url: &str,
	target: &Schema,
	allow_destructive: bool,
) -> Result<Option<Squash>> {
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	let runtime = super::runtime()?;
	let target = target.without_rename_hints();
	let prepared = runtime.block_on(async {
		let db = Db::connect(url).await?;

		if db.pushed_schema().await?.is_none() {
			return anyhow::Ok(None);
		}

		let current = holds(&db, &history).await?;
		let plan = leptoskit_db_schema::diff(&current, &target, &mut RequireHints)?;

		anyhow::ensure!(
			plan.destructive_steps().next().is_none() || allow_destructive,
			"the development database's pushes are behind the tables, and catching up deletes \
			 data:\n{}\npass --allow-destructive if that's intended",
			described(&plan)
				.iter()
				.map(|step| format!("  {step}"))
				.collect::<Vec<_>>()
				.join("\n")
		);
		let rest = (!plan.is_empty()).then(|| {
			Migration::new(
				"push",
				leptoskit_db_schema::render(&plan, &current, &target).sql(),
			)
		});
		Ok(Some((db, rest)))
	})?;

	Ok(prepared.map(|(db, rest)| Squash { db, rest, runtime }))
}

impl Squash {
	/// Catches the database up with the tables, then records `migration`,
	/// just written, as applied, and forgets the pushes.
	pub(crate) fn adopt(self, migration: &Migration, target: &Schema) -> Result<()> {
		self.runtime.block_on(async {
			if let Some(rest) = &self.rest {
				self.db
					.push(rest, &target.without_rename_hints().to_json())
					.await?;
			}

			self.db.adopt(migration).await?;
			anyhow::Ok(())
		})
	}
}
