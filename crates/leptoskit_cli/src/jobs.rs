//! `leptoskit jobs`: lists, retries, and cancels an app's background jobs,
//! straight from the `rpc` pack's `jobs` table of any deployment's database.

use std::fmt::Write as _;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use clap::Subcommand;
use leptoskit_db::Column;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;

/// What `leptoskit jobs` does.
#[derive(Debug, Subcommand)]
pub enum JobsCommand {
	/// Lists jobs, newest first: their name, queue, state, attempts, when
	/// they run next, and why they last failed.
	List {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
		/// Only jobs in this state: `pending`, `running`, `done`, `failed`,
		/// or `cancelled`.
		#[arg(long)]
		state: Option<String>,
		/// How many to list.
		#[arg(long, default_value_t = 50)]
		limit: u32,
	},
	/// Runs a failed or cancelled job again from the start, now, with its
	/// attempts reset and no deadline.
	Retry {
		/// The job's id, as `jobs list` shows it.
		id: String,
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
	},
	/// Cancels a job: a waiting one never starts, and a running one stops at
	/// its next step.
	Cancel {
		/// The job's id, as `jobs list` shows it.
		id: String,
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
	},
}

/// The states a job can be in.
const STATES: [&str; 5] = ["pending", "running", "done", "failed", "cancelled"];

/// Runs `leptoskit jobs`.
///
/// # Errors
///
/// When there's no database URL, the database can't be reached, or the job
/// isn't there or isn't in a state the command applies to.
pub fn run(command: JobsCommand) -> Result<()> {
	let url = match &command {
		JobsCommand::List { url, .. }
		| JobsCommand::Retry { url, .. }
		| JobsCommand::Cancel { url, .. } => database_url(url.clone())?,
	};
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(&url)
			.await
			.with_context(|| "couldn't connect to the database")?;
		let output = match command {
			JobsCommand::List { state, limit, .. } => list(&db, state.as_deref(), limit).await?,
			JobsCommand::Retry { id, .. } => retry(&db, &id).await?,
			JobsCommand::Cancel { id, .. } => cancel(&db, &id).await?,
		};
		print!("{output}");
		Ok(())
	})
}

/// `--url`, or the variable the project's `db.url_env` names.
fn database_url(url: Option<String>) -> Result<String> {
	if let Some(url) = url {
		return Ok(url);
	}
	let project = crate::db::Project::load()
		.context("pass --url with the database's URL, or run this in a project with a database")?;
	std::env::var(&project.config.url_env).with_context(|| {
		format!(
			"set {} to the database URL, or pass --url",
			project.config.url_env
		)
	})
}

/// The jobs table, quoted for the database.
fn table(db: &Db) -> &'static str {
	match db.dialect() {
		Dialect::Postgres => "\"rpc\".\"jobs\"",
		Dialect::Sqlite => "\"rpc_jobs\"",
	}
}

/// Placeholder `n` for the database.
fn placeholder(db: &Db, n: usize) -> String {
	match db.dialect() {
		Dialect::Postgres => format!("${n}"),
		Dialect::Sqlite => format!("?{n}"),
	}
}

fn statement(sql: String, args: Vec<Value>) -> Statement {
	Statement { sql, args }
}

/// A job's id from the command line.
fn job_id(id: &str) -> Result<Value> {
	let id: Id<()> = id
		.trim()
		.parse()
		.ok()
		.with_context(|| format!("`{id}` isn't a job id; `jobs list` shows them"))?;
	Ok(Value::Uuid(id.uuid()))
}

fn text(value: Value) -> String {
	match value {
		Value::Null => String::new(),
		Value::Text(text) => text,
		Value::Int(value) => value.to_string(),
		Value::Uuid(uuid) => uuid.to_string(),
		// SQLite keeps a UUID as its 16 bytes.
		Value::Bytes(bytes) if bytes.len() == 16 => {
			let hex = bytes.iter().fold(String::new(), |mut hex, byte| {
				let _ = write!(hex, "{byte:02x}");
				hex
			});
			format!(
				"{}-{}-{}-{}-{}",
				&hex[..8],
				&hex[8..12],
				&hex[12..16],
				&hex[16..20],
				&hex[20..]
			)
		}
		other => {
			Timestamp::from_value(other.clone()).map_or_else(
				|_| format!("{other:?}"),
				|at| at.format("%Y-%m-%d %H:%M:%S").to_string(),
			)
		}
	}
}

/// The newest `limit` jobs, in `state` if given, as a table.
pub(crate) async fn list(db: &Db, state: Option<&str>, limit: u32) -> Result<String> {
	let mut sql = format!(
		"SELECT id, name, queue, state, attempts, max_attempts, run_at, error FROM {}",
		table(db)
	);
	let mut args = Vec::new();
	if let Some(state) = state {
		if !STATES.contains(&state) {
			bail!("`{state}` isn't a state; use one of {}", STATES.join(", "));
		}
		let _ = write!(sql, " WHERE state = {}", placeholder(db, 1));
		args.push(Value::Text(state.to_owned()));
	}
	let _ = write!(sql, " ORDER BY created_at DESC LIMIT {}", limit.max(1));
	let rows = db.fetch(statement(sql, args)).await?;
	if rows.is_empty() {
		return Ok("no jobs\n".to_owned());
	}
	let mut out = String::new();
	for row in rows {
		let mut columns = row.into_iter().map(text);
		let mut next = || columns.next().unwrap_or_default();
		let (id, name, queue, state) = (next(), next(), next(), next());
		let (attempts, max_attempts, run_at, error) = (next(), next(), next(), next());
		let _ = write!(
			out,
			"{id}  {name}  {queue}  {state}  {attempts}/{max_attempts}  runs {run_at}"
		);
		if !error.is_empty() {
			let error: String = error.chars().take(120).collect();
			let _ = write!(out, "  {error}");
		}
		out.push('\n');
	}
	Ok(out)
}

/// Runs a failed or cancelled job again.
pub(crate) async fn retry(db: &Db, id: &str) -> Result<String> {
	let sql = format!(
		"UPDATE {} SET state = {}, attempts = 0, run_at = {}, deadline_at = NULL, finished_at = \
		 NULL, locked_until = NULL, cancel_requested = {}, failure_pending = {} WHERE id = {} AND \
		 state IN ({}, {})",
		table(db),
		placeholder(db, 1),
		placeholder(db, 2),
		placeholder(db, 3),
		placeholder(db, 3),
		placeholder(db, 4),
		placeholder(db, 5),
		placeholder(db, 6),
	);
	let changed = db
		.execute(statement(
			sql,
			vec![
				Value::Text("pending".into()),
				Value::Timestamp(leptoskit_db::now()),
				Value::Bool(false),
				job_id(id)?,
				Value::Text("failed".into()),
				Value::Text("cancelled".into()),
			],
		))
		.await?;
	if changed == 1 {
		return Ok(format!("job {} will run again\n", id.trim()));
	}
	bail!(
		"{}",
		missing_or(db, id, "only failed or cancelled jobs can be retried").await?
	)
}

/// Cancels a waiting job, or asks a running one to stop.
pub(crate) async fn cancel(db: &Db, id: &str) -> Result<String> {
	let waiting = db
		.execute(statement(
			format!(
				"UPDATE {} SET state = {}, cancel_requested = {}, finished_at = {} WHERE id = {} \
				 AND state = {}",
				table(db),
				placeholder(db, 1),
				placeholder(db, 2),
				placeholder(db, 3),
				placeholder(db, 4),
				placeholder(db, 5),
			),
			vec![
				Value::Text("cancelled".into()),
				Value::Bool(true),
				Value::Timestamp(leptoskit_db::now()),
				job_id(id)?,
				Value::Text("pending".into()),
			],
		))
		.await?;
	if waiting == 1 {
		return Ok(format!("job {} is cancelled\n", id.trim()));
	}
	let running = db
		.execute(statement(
			format!(
				"UPDATE {} SET cancel_requested = {} WHERE id = {} AND state = {}",
				table(db),
				placeholder(db, 1),
				placeholder(db, 2),
				placeholder(db, 3),
			),
			vec![
				Value::Bool(true),
				job_id(id)?,
				Value::Text("running".into()),
			],
		))
		.await?;
	if running == 1 {
		return Ok(format!("job {} will stop at its next step\n", id.trim()));
	}
	bail!(
		"{}",
		missing_or(db, id, "only waiting or running jobs can be cancelled").await?
	)
}

/// Why a command changed nothing: there's no such job, or it's in another
/// state.
async fn missing_or(db: &Db, id: &str, otherwise: &str) -> Result<String> {
	let rows = db
		.fetch(statement(
			format!(
				"SELECT state FROM {} WHERE id = {}",
				table(db),
				placeholder(db, 1)
			),
			vec![job_id(id)?],
		))
		.await?;
	Ok(
		match rows
			.into_iter()
			.next()
			.and_then(|row| row.into_iter().next())
		{
			None => format!("there's no job {}", id.trim()),
			Some(state) => format!("{otherwise}; this one is {}", text(state)),
		},
	)
}

#[cfg(test)]
#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use leptoskit_rpc as rpc;

	use super::*;

	/// A job for the commands to find.
	#[rpc::job(retries = 0)]
	pub async fn import(_cx: &rpc::JobCx, file: String) -> rpc::Result<()> {
		Err(rpc::Error::permanent(format!("{file} is empty")))
	}

	#[allow(
		clippy::cognitive_complexity,
		reason = "one story: a job listed, retried, and cancelled"
	)]
	async fn commands_list_retry_and_cancel(db: &Db) {
		assert_eq!(list(db, None, 10).await.unwrap(), "no jobs\n");
		let failing = rpc::enqueue(
			db,
			Import {
				file: "a.csv".into(),
			},
		)
		.await
		.unwrap();
		rpc::testing::TestJobs::new(db).run_jobs_until_idle().await;
		let waiting = rpc::enqueue(
			db,
			Import {
				file: "b.csv".into(),
			},
		)
		.after(Duration::from_secs(3_600))
		.await
		.unwrap();

		let all = list(db, None, 10).await.unwrap();
		assert_eq!(all.lines().count(), 2, "{all}");
		assert!(
			all.contains(&format!("{failing}  import  default  failed  1/1")),
			"{all}"
		);
		assert!(all.contains("a.csv is empty"), "{all}");
		let failed = list(db, Some("failed"), 10).await.unwrap();
		assert_eq!(failed.lines().count(), 1, "{failed}");
		assert!(list(db, Some("stuck"), 10).await.is_err());

		assert!(
			retry(db, &waiting.to_string())
				.await
				.unwrap_err()
				.to_string()
				.contains("this one is pending")
		);
		assert!(retry(db, "nope").await.is_err());
		assert!(
			retry(db, &Id::<()>::new().to_string())
				.await
				.unwrap_err()
				.to_string()
				.contains("there's no job")
		);
		assert!(
			retry(db, &failing.to_string())
				.await
				.unwrap()
				.contains("will run again")
		);
		assert_eq!(
			list(db, Some("pending"), 10).await.unwrap().lines().count(),
			2
		);

		assert!(
			cancel(db, &waiting.to_string())
				.await
				.unwrap()
				.contains("is cancelled")
		);
		assert!(
			cancel(db, &waiting.to_string())
				.await
				.unwrap_err()
				.to_string()
				.contains("this one is cancelled")
		);
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn jobs_commands_work_on_sqlite() {
		commands_list_retry_and_cancel(&leptoskit_db::testing::sqlite().await).await;
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn jobs_commands_work_on_postgres() {
		let Some(cluster) = postgres::PostgresCluster::start() else {
			return;
		};
		let db = Db::connect(&cluster.database()).await.unwrap();
		leptoskit_db::testing::create_tables(&db).await.unwrap();
		commands_list_retry_and_cancel(&db).await;
	}

	#[test]
	fn a_url_or_a_project_is_needed() {
		assert_eq!(
			database_url(Some("sqlite::memory:".into())).unwrap(),
			"sqlite::memory:"
		);
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("jobs.db");
		std::fs::File::create(&path).unwrap();
		let url = format!("sqlite://{}", path.display());
		crate::db::runtime().unwrap().block_on(async {
			let db = Db::connect(&url).await.unwrap();
			leptoskit_db::testing::create_tables(&db).await.unwrap();
		});
		run(JobsCommand::List {
			url: Some(url.clone()),
			state: None,
			limit: 5,
		})
		.unwrap();
		assert!(
			run(JobsCommand::Cancel {
				id: Id::<()>::new().to_string(),
				url: Some(url.clone()),
			})
			.is_err()
		);
		assert!(
			run(JobsCommand::Retry {
				id: "x".into(),
				url: Some(url),
			})
			.is_err()
		);
	}
}
