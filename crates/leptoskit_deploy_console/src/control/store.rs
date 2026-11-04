use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit::analytics::Snapshot;
use leptoskit::analytics::Window;
use rusqlite::OptionalExtension as _;
use rusqlite::params;
use sha2::Digest as _;

use super::*;

const UP: &str = include_str!("migrations/001.up.sql");
const DOWN: &str = include_str!("migrations/001.down.sql");
const RETENTION: u64 = 7 * 86400;

/// A private, bounded SQLite journal. All read/modify/write operations use
/// SQLite transactions, including calls made through separate connections.
#[derive(Clone)]
pub struct Store(Arc<Mutex<rusqlite::Connection>>);

impl Store {
	pub fn open(path: &Path) -> Result<Self> {
		let parent = path.parent().context("database needs a parent directory")?;
		let existed = parent.exists();
		std::fs::create_dir_all(parent)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			if !existed {
				std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
			}
			ensure!(
				std::fs::metadata(parent)?
					.permissions()
					.mode()
					.trailing_zeros()
					>= 6,
				"management data needs a private directory"
			);
		}
		let mut options = std::fs::OpenOptions::new();
		options.create(true).write(true).truncate(false);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt as _;
			options.mode(0o600);
		}
		drop(options.open(path)?);
		let mut db = rusqlite::Connection::open(path)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
		}
		db.busy_timeout(std::time::Duration::from_secs(5))?;
		db.execute_batch(
			"PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA \
			 max_page_count=262144;",
		)?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		tx.execute_batch(
			"CREATE TABLE IF NOT EXISTS _lk_migrations(version INTEGER PRIMARY KEY, checksum TEXT \
			 NOT NULL)",
		)?;
		let current: Option<(u32, String)> = tx
			.query_row(
				"SELECT version, checksum FROM _lk_migrations ORDER BY version DESC LIMIT 1",
				[],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.optional()?;
		let checksum = format!("{:x}", sha2::Sha256::digest(UP.as_bytes()));
		if let Some((version, applied)) = current {
			ensure!(
				version == 1 && applied == checksum,
				"unsupported or modified management database migration"
			);
		} else {
			tx.execute_batch(UP)?;
			tx.execute("INSERT INTO _lk_migrations VALUES(1, ?1)", [&checksum])?;
		}
		tx.commit()?;
		Ok(Self(Arc::new(Mutex::new(db))))
	}

	pub(crate) fn db(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
		self.0
			.lock()
			.map_err(|_| anyhow::anyhow!("management database lock failed"))
	}

	pub fn register(&self, project: &Project) -> Result<()> {
		ensure!(
			identifier(&project.id)
				&& !project.environment.is_empty()
				&& project.environment.len() <= 128
				&& project
					.environment
					.bytes()
					.all(|byte| (32..127).contains(&byte))
				&& project.root.is_absolute(),
			"invalid registered project"
		);
		let mut db = self.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let existing: Option<(String, String)> = tx
			.query_row(
				"SELECT root,environment FROM projects WHERE id=?1",
				[&project.id],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()?;
		let root = project
			.root
			.to_str()
			.context("repository path must be UTF-8")?;
		if let Some((old, environment)) = existing {
			ensure!(
				old == root && environment == project.environment,
				"project identity or root changed; register a distinct project ID"
			);
		} else {
			tx.execute(
				"INSERT INTO projects(id,environment,root) VALUES(?1,?2,?3)",
				params![project.id, project.environment, root],
			)?;
		}
		tx.commit()?;
		Ok(())
	}

	pub fn projects(&self) -> Result<Vec<Project>> {
		let db = self.db()?;
		let mut query =
			db.prepare("SELECT id,environment,root,revision FROM projects ORDER BY id")?;
		Ok(query
			.query_map([], |r| {
				Ok(Project {
					id: r.get(0)?,
					environment: r.get(1)?,
					root: std::path::PathBuf::from(r.get::<_, String>(2)?),
					revision: r.get::<_, i64>(3)? as u64,
				})
			})?
			.collect::<rusqlite::Result<_>>()?)
	}

	pub fn enqueue(&self, project: &str, intent: &Intent) -> Result<Operation> {
		ensure!(
			identifier(&intent.idempotency_key) && intent.action.valid(),
			"invalid_intent"
		);
		let request = serde_json::to_string(intent)?;
		let mut db = self.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let duplicate: Option<(String, String)> = tx
			.query_row(
				"SELECT id,request FROM operations WHERE project=?1 AND idempotency=?2",
				params![project, intent.idempotency_key],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()?;
		if let Some((id, previous)) = duplicate {
			ensure!(previous == request, "idempotency_conflict");
			return operation(&tx, &id)?.context("operation missing");
		}
		let revision: i64 = tx
			.query_row(
				"SELECT revision FROM projects WHERE id=?1",
				[project],
				|r| r.get(0),
			)
			.context("unknown_project")?;
		ensure!(
			revision as u64 == intent.expected_revision,
			"revision_conflict"
		);
		let busy: bool = tx.query_row(
			"SELECT EXISTS(SELECT 1 FROM operations WHERE project=?1 AND phase IN \
			 ('queued','running'))",
			[project],
			|r| r.get(0),
		)?;
		ensure!(!busy, "operation_in_progress");
		if let Action::Apply { plan_id } = &intent.action {
			let plan: Option<(String, String)> = tx
				.query_row(
					"SELECT id,phase FROM operations WHERE project=?1 AND \
					 json_extract(request,'$.action.kind')='plan' ORDER BY rowid DESC LIMIT 1",
					[project],
					|r| Ok((r.get(0)?, r.get(1)?)),
				)
				.optional()?;
			ensure!(
				plan == Some((plan_id.clone(), "succeeded".into())),
				"plan_is_not_current"
			);
			let used: bool = tx.query_row(
				"SELECT EXISTS(SELECT 1 FROM operations WHERE project=?1 AND \
				 json_extract(request,'$.action.plan_id')=?2)",
				params![project, plan_id],
				|r| r.get(0),
			)?;
			ensure!(!used, "plan_already_consumed");
		}
		let id = random_key()?;
		let at = now() as i64;
		tx.execute(
			"UPDATE projects SET revision=revision+1 WHERE id=?1",
			[project],
		)?;
		tx.execute(
			"INSERT INTO \
			 operations(id,project,idempotency,request,revision,phase,created,updated,message) \
			 VALUES(?1,?2,?3,?4,?5,'queued',?6,?6,'Queued')",
			params![
				id,
				project,
				intent.idempotency_key,
				request,
				revision + 1,
				at
			],
		)?;
		let result = operation(&tx, &id)?.context("operation missing")?;
		tx.commit()?;
		Ok(result)
	}

	pub fn operation(&self, id: &str) -> Result<Option<Operation>> {
		let db = self.db()?;
		operation(&db, id)
	}

	pub fn operations(&self) -> Result<Vec<Operation>> {
		let db = self.db()?;
		let mut query = db.prepare("SELECT id FROM operations ORDER BY rowid DESC LIMIT 100")?;
		let ids: Vec<String> = query
			.query_map([], |r| r.get(0))?
			.collect::<rusqlite::Result<_>>()?;
		ids.into_iter()
			.map(|id| operation(&db, &id)?.context("operation missing"))
			.collect()
	}

	pub fn interrupt_running(&self) -> Result<()> {
		self.db()?.execute(
			"UPDATE operations SET phase='interrupted',updated=?1,message='Runner restarted; \
			 reconcile infrastructure and the shared operation owner before retrying' WHERE \
			 phase='running'",
			[now() as i64],
		)?;
		Ok(())
	}

	pub fn claim(&self) -> Result<Option<Operation>> {
		self.claim_project(None)
	}

	/// Hosted workers can only claim their registered project.
	pub fn claim_project(&self, project: Option<&str>) -> Result<Option<Operation>> {
		let mut db = self.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let id: Option<String> = tx
			.query_row(
				"SELECT id FROM operations WHERE phase='queued' AND (?1 IS NULL OR project=?1) \
				 ORDER BY rowid LIMIT 1",
				[project],
				|r| r.get(0),
			)
			.optional()?;
		let Some(id) = id else {
			return Ok(None);
		};
		tx.execute(
			"UPDATE operations SET phase='running',updated=?1,message='Running on the registered \
			 deployment worker' WHERE id=?2",
			params![now() as i64, id],
		)?;
		let result = operation(&tx, &id)?;
		tx.commit()?;
		Ok(result)
	}

	pub fn finish(&self, id: &str, result: &RunResult) -> Result<()> {
		ensure!(
			result.message.len() <= 2048
				&& serde_json::to_vec(&result.details)?.len() <= 128 * 1024,
			"operation result exceeds journal limit"
		);
		let changed = self.db()?.execute(
			"UPDATE operations SET phase=?1,updated=?2,message=?3,result=?4 WHERE id=?5 AND \
			 phase='running'",
			params![
				if result.succeeded {
					"succeeded"
				} else {
					"failed"
				},
				now() as i64,
				result.message,
				serde_json::to_string(&result.details)?,
				id
			],
		)?;
		ensure!(changed == 1, "operation lost its running state");
		Ok(())
	}

	pub fn progress(&self, id: &str, message: &str) -> Result<()> {
		ensure!(
			message.len() <= 2048,
			"progress message exceeds journal limit"
		);
		self.db()?.execute(
			"UPDATE operations SET message=?1,updated=?2 WHERE id=?3 AND phase='running'",
			params![message, now() as i64, id],
		)?;
		Ok(())
	}

	pub fn maintain(&self) -> Result<()> {
		let db = self.db()?;
		db.execute(
			"DELETE FROM metric_windows WHERE start<?1",
			[now().saturating_sub(RETENTION) as i64],
		)?;
		db.execute(
			"DELETE FROM producers WHERE seen<?1",
			[now().saturating_sub(RETENTION) as i64],
		)?;
		db.execute("DELETE FROM tokens WHERE expires<?1", [now() as i64])?;
		db.execute(
			"DELETE FROM operations WHERE phase NOT IN ('queued','running') AND rowid IN (SELECT \
			 rowid FROM operations ORDER BY rowid DESC LIMIT -1 OFFSET 10000)",
			[],
		)?;
		Ok(())
	}

	pub fn issue(
		&self,
		scope: Scope,
		project: Option<&str>,
		lifetime: u64,
		session: bool,
	) -> Result<String> {
		ensure!(
			lifetime > 0 && lifetime <= 366 * 86400,
			"invalid token lifetime"
		);
		ensure!(
			scope == Scope::Owner || project.is_some(),
			"non-owner tokens need a project"
		);
		let key = random_key()?;
		self.db()?.execute(
			"INSERT INTO tokens(hash,scope,project,expires,session) VALUES(?1,?2,?3,?4,?5)",
			params![
				sha2::Sha256::digest(key.as_bytes()).as_slice(),
				serde_json::to_string(&scope)?,
				project,
				(now() + lifetime) as i64,
				session
			],
		)?;
		Ok(key)
	}

	pub(crate) fn authenticate(&self, key: &str, session: bool) -> Result<Option<Principal>> {
		if key.len() != 64 {
			return Ok(None);
		}
		self.db()?
			.query_row(
				"SELECT scope,project,session FROM tokens WHERE hash=?1 AND expires>?2 AND \
				 session=?3 AND revoked=0",
				params![
					sha2::Sha256::digest(key.as_bytes()).as_slice(),
					now() as i64,
					session
				],
				|r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
			)
			.optional()?
			.map(|(scope, project, session)| {
				Ok(Principal {
					scope: serde_json::from_str(&scope)?,
					project,
					session,
				})
			})
			.transpose()
	}

	pub fn revoke(&self, key: &str) -> Result<()> {
		self.db()?.execute(
			"UPDATE tokens SET revoked=1 WHERE hash=?1",
			[sha2::Sha256::digest(key.as_bytes()).as_slice()],
		)?;
		Ok(())
	}

	pub(crate) fn install_ingest_key(&self, project: &str, key: &str) -> Result<()> {
		self.db()?.execute(
			"INSERT OR IGNORE INTO tokens(hash,scope,project,expires,session) \
			 VALUES(?1,?2,?3,?4,0)",
			params![
				sha2::Sha256::digest(key.as_bytes()).as_slice(),
				serde_json::to_string(&Scope::Ingest)?,
				project,
				i64::MAX
			],
		)?;
		let revoked: bool = self.db()?.query_row(
			"SELECT revoked FROM tokens WHERE hash=?1",
			[sha2::Sha256::digest(key.as_bytes()).as_slice()],
			|r| r.get(0),
		)?;
		ensure!(
			!revoked,
			"managed analytics credential was revoked; rotate the owner key before redeploying"
		);
		Ok(())
	}

	pub fn ingest(&self, project: &str, snapshot: &Snapshot) -> Result<()> {
		validate(snapshot)?;
		let mut db = self.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let environment: String = tx.query_row(
			"SELECT environment FROM projects WHERE id=?1",
			[project],
			|r| r.get(0),
		)?;
		ensure!(
			snapshot.resource.service == project && snapshot.resource.environment == environment,
			"analytics_identity_mismatch"
		);
		let producer = format!(
			"{}:{}",
			snapshot.resource.instance, snapshot.started_unix_seconds
		);
		tx.execute(
			"DELETE FROM metric_windows WHERE start<?1",
			[now().saturating_sub(RETENTION) as i64],
		)?;
		tx.execute(
			"DELETE FROM producers WHERE seen<?1",
			[now().saturating_sub(RETENTION) as i64],
		)?;
		tx.execute("DELETE FROM tokens WHERE expires<?1", [now() as i64])?;
		let count: i64 = tx.query_row(
			"SELECT COUNT(*) FROM producers WHERE project=?1",
			[project],
			|r| r.get(0),
		)?;
		ensure!(
			count < 1024
				|| tx.query_row(
					"SELECT EXISTS(SELECT 1 FROM producers WHERE project=?1 AND producer=?2)",
					params![project, producer],
					|r| r.get::<_, bool>(0)
				)?,
			"analytics_instance_quota"
		);
		let previous: Option<String> = tx
			.query_row(
				"SELECT release FROM producers WHERE project=?1 AND producer=?2",
				params![project, producer],
				|r| r.get(0),
			)
			.optional()?;
		ensure!(
			previous
				.as_ref()
				.is_none_or(|release| release == &snapshot.resource.release),
			"analytics_producer_release_changed"
		);
		let sample = serde_json::to_string(&HostReport {
			instance: snapshot.resource.instance.clone(),
			release: snapshot.resource.release.clone(),
			seen: snapshot.observed_unix_seconds,
			host: snapshot.host.clone(),
			export: snapshot.export.clone(),
		})?;
		tx.execute(
			"INSERT INTO producers VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project,producer) DO UPDATE \
			 SET seen=excluded.seen,sample=excluded.sample WHERE excluded.seen>=producers.seen",
			params![
				project,
				producer,
				snapshot.resource.release,
				snapshot.observed_unix_seconds as i64,
				sample
			],
		)?;
		for series in &snapshot.series {
			let key = serde_json::to_string(&series.key)?;
			for window in &series.windows {
				tx.execute(
					"INSERT INTO metric_windows VALUES(?1,?2,?3,?4,?5,?6,?7) ON \
					 CONFLICT(project,producer,series,start) DO UPDATE SET \
					 count=excluded.count,payload=excluded.payload WHERE \
					 excluded.count>=metric_windows.count",
					params![
						project,
						producer,
						snapshot.resource.release,
						key,
						window.start_unix_seconds as i64,
						window.count as i64,
						serde_json::to_string(window)?
					],
				)?;
			}
		}
		let rows: i64 = tx.query_row(
			"SELECT COUNT(*) FROM metric_windows WHERE project=?1",
			[project],
			|r| r.get(0),
		)?;
		if rows > 100_000 {
			tx.execute(
				"DELETE FROM metric_windows WHERE rowid IN (SELECT rowid FROM metric_windows \
				 WHERE project=?1 ORDER BY start,rowid LIMIT ?2)",
				params![project, rows - 100_000],
			)?;
		}
		tx.commit()?;
		Ok(())
	}

	pub fn fleet(&self, project: &str, from: u64, to: u64) -> Result<Fleet> {
		ensure!(to >= from && to - from <= 86400, "analytics_range_limit");
		let from = from.max(now().saturating_sub(RETENTION));
		ensure!(to >= from, "analytics_range_expired");
		let db = self.db()?;
		let mut query = db.prepare(
			"SELECT release,series,payload FROM metric_windows WHERE project=?1 AND start>=?2 AND \
			 start<?3 ORDER BY start LIMIT 100001",
		)?;
		let rows = query
			.query_map(params![project, from as i64, to as i64], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, String>(2)?,
				))
			})?
			.collect::<rusqlite::Result<Vec<_>>>()?;
		ensure!(rows.len() <= 100_000, "analytics_query_limit");
		let mut groups: BTreeMap<(String, String), BTreeMap<u64, Window>> = BTreeMap::new();
		for (release, key, payload) in rows {
			let window: Window = serde_json::from_str(&payload)?;
			let target = groups
				.entry((release, key))
				.or_default()
				.entry(window.start_unix_seconds)
				.or_insert_with(|| {
					Window {
						start_unix_seconds: window.start_unix_seconds,
						..Window::default()
					}
				});
			target.count = target.count.saturating_add(window.count);
			target.duration_sum_ms += window.duration_sum_ms;
			target.duration_max_ms = target.duration_max_ms.max(window.duration_max_ms);
			for (total, value) in target.latency.iter_mut().zip(window.latency) {
				*total = total.saturating_add(value);
			}
		}
		let series = groups
			.into_iter()
			.map(|((release, key), windows)| {
				Ok(FleetSeries {
					release,
					key: serde_json::from_str(&key)?,
					windows: windows.into_values().collect(),
				})
			})
			.collect::<Result<_>>()?;
		let instances: i64 = db.query_row(
			"SELECT COUNT(*) FROM producers WHERE project=?1 AND seen>=?2",
			params![project, from as i64],
			|r| r.get(0),
		)?;
		let stale: i64 = db.query_row(
			"SELECT COUNT(*) FROM producers WHERE project=?1 AND seen>=?2 AND seen<?3",
			params![project, from as i64, now().saturating_sub(120) as i64],
			|r| r.get(0),
		)?;
		let retained_from: Option<i64> = db.query_row(
			"SELECT MIN(start) FROM metric_windows WHERE project=?1",
			[project],
			|r| r.get(0),
		)?;
		let mut query = db.prepare(
			"SELECT sample FROM producers WHERE project=?1 AND seen>=?2 ORDER BY seen DESC LIMIT \
			 1024",
		)?;
		let hosts = query
			.query_map(params![project, from as i64], |r| r.get::<_, String>(0))?
			.map(|row| Ok(serde_json::from_str(&row?)?))
			.collect::<Result<Vec<HostReport>>>()?;
		Ok(Fleet {
			hosts,
			retained_from: retained_from.map(|n| n as u64),
			from,
			to,
			instances: instances as usize,
			stale_instances: stale as usize,
			series,
			limitations: vec![
				"Only reporting instances are counted; compare with deployment inventory for \
				 missing hosts."
					.into(),
				"Current and startup windows can be partial. Browser reports remain untrusted."
					.into(),
				"Up to seven days and 100,000 contribution windows per project; the oldest \
				 windows expire first. Query ranges are limited to 24 hours."
					.into(),
			],
		})
	}

	pub fn backup(&self, path: &Path) -> Result<()> {
		ensure!(!path.exists(), "backup already exists");
		let parent = path.parent().context("backup needs a parent directory")?;
		let temporary = tempfile::NamedTempFile::new_in(parent)?;
		let db = self.db()?;
		db.backup(rusqlite::MAIN_DB, temporary.path(), None)?;
		temporary.as_file().sync_all()?;
		temporary.persist_noclobber(path)?;
		Ok(())
	}

	/// Offline schema reversal. The caller must hold the service directory lock
	/// and take an explicit backup first; the first down migration deletes
	/// history.
	pub fn rollback(&self, backup: &Path) -> Result<()> {
		self.backup(backup)?;
		let mut db = self.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		tx.execute_batch(DOWN)?;
		tx.execute("DELETE FROM _lk_migrations WHERE version=1", [])?;
		tx.commit()?;
		Ok(())
	}

	/// Offline restore into a validated candidate, retaining a consistent
	/// previous database. Revokes restored credentials, including browser
	/// sessions.
	pub fn restore(database: &Path, source: &Path, previous: &Path) -> Result<()> {
		let parent = database
			.parent()
			.context("database needs a parent directory")?;
		ensure!(
			source != database && previous != database && source != previous,
			"restore paths must be distinct"
		);
		let candidate = tempfile::NamedTempFile::new_in(parent)?;
		std::fs::copy(source, candidate.path())?;
		{
			let restored = Self::open(candidate.path())?;
			let db = restored.db()?;
			let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
			ensure!(
				check == "ok",
				"management backup failed integrity verification"
			);
			let violations: u64 = db
				.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
					r.get::<_, i64>(0)
				})
				.map(|n| n as u64)?;
			ensure!(
				violations == 0,
				"management backup has invalid foreign keys"
			);
			db.execute("UPDATE tokens SET revoked=1", [])?;
		}
		if database.exists() {
			Self::open(database)?.backup(previous)?;
		}
		for suffix in ["-wal", "-shm"] {
			let sidecar = std::path::PathBuf::from(format!("{}{suffix}", database.display()));
			if sidecar.exists() {
				std::fs::remove_file(sidecar)?;
			}
		}
		candidate.as_file().sync_all()?;
		candidate.persist(database)?;
		Ok(())
	}
}

fn operation(db: &rusqlite::Connection, id: &str) -> Result<Option<Operation>> {
	let mut query = db.prepare(
		"SELECT sequence,phase,message,created FROM operation_events WHERE operation=?1 ORDER BY \
		 sequence DESC LIMIT 64",
	)?;
	let mut events = query
		.query_map([id], |r| {
			Ok((
				r.get::<_, i64>(0)?,
				r.get::<_, String>(1)?,
				r.get::<_, String>(2)?,
				r.get::<_, i64>(3)?,
			))
		})?
		.map(|row| {
			let (sequence, phase, message, created) = row?;
			Ok(Event {
				sequence: sequence as u64,
				phase: serde_json::from_value(serde_json::json!(phase))?,
				message,
				created: created as u64,
			})
		})
		.collect::<Result<Vec<_>>>()?;
	events.reverse();
	let row = db
		.query_row(
			"SELECT project,request,revision,phase,created,updated,message,result FROM operations \
			 WHERE id=?1",
			[id],
			|r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, i64>(2)?,
					r.get::<_, String>(3)?,
					r.get::<_, i64>(4)?,
					r.get::<_, i64>(5)?,
					r.get::<_, String>(6)?,
					r.get::<_, String>(7)?,
				))
			},
		)
		.optional()?;
	row.map(
		|(project, request, revision, phase, created, updated, message, result)| {
			Ok(Operation {
				events,
				id: id.into(),
				project,
				intent: serde_json::from_str(&request)?,
				revision: revision as u64,
				phase: serde_json::from_value(serde_json::json!(phase))?,
				created: created as u64,
				updated: updated as u64,
				message,
				result: serde_json::from_str(&result)?,
			})
		},
	)
	.transpose()
}

fn validate(snapshot: &Snapshot) -> Result<()> {
	let at = now();
	ensure!(
		snapshot.format_version == 1
			&& snapshot.window_seconds == 60
			&& snapshot.series.len() <= 512
			&& snapshot.recent.len() <= 256,
		"invalid_analytics_format"
	);
	ensure!(
		snapshot.started_unix_seconds <= snapshot.observed_unix_seconds
			&& snapshot.observed_unix_seconds <= at + 120
			&& snapshot.observed_unix_seconds >= at.saturating_sub(RETENTION),
		"analytics_clock_skew"
	);
	for value in [
		&snapshot.resource.service,
		&snapshot.resource.environment,
		&snapshot.resource.release,
		&snapshot.resource.instance,
	] {
		ensure!(
			!value.is_empty()
				&& value.len() <= 128
				&& value.bytes().all(|b| (32..127).contains(&b)),
			"invalid_analytics_identity"
		);
	}
	for series in &snapshot.series {
		ensure!(
			series.key.operation.len() <= 128
				&& !series.key.operation.is_empty()
				&& series
					.key
					.operation
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"/:-_.*{}".contains(&b))
				&& series.windows.len() <= 60,
			"invalid_analytics_series"
		);
		for window in &series.windows {
			ensure!(
				window.start_unix_seconds % 60 == 0
					&& window.start_unix_seconds >= at.saturating_sub(RETENTION)
					&& window.start_unix_seconds <= snapshot.observed_unix_seconds,
				"invalid_analytics_window"
			);
			ensure!(
				window.count <= 1_000_000_000_000
					&& window
						.latency
						.iter()
						.try_fold(0_u64, |total, n| total.checked_add(*n))
						== Some(window.count)
					&& window.duration_sum_ms.is_finite()
					&& window.duration_max_ms.is_finite()
					&& window.duration_sum_ms >= 0.0
					&& window.duration_max_ms >= 0.0
					&& window.duration_max_ms <= 86_400_000.0
					&& window.duration_sum_ms <= window.count as f64 * 86_400_000.0,
				"invalid_analytics_values"
			);
		}
	}
	Ok(())
}
