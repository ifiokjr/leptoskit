use rusqlite::OptionalExtension as _;
use rusqlite::params;

use super::*;

/// A pilot account is operator-enrolled; there is no public unauthenticated key
/// minting.
#[derive(Clone)]
pub struct Accounts {
	pub(crate) meta: Store,
	pub(crate) root: std::path::PathBuf,
}
impl Accounts {
	pub fn open(root: &Path) -> Result<Self> {
		let meta = Store::open(&root.join("accounts.sqlite3"))?;
		{
			let mut db = meta.db()?;
			let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
			tx.execute_batch(
				"CREATE TABLE IF NOT EXISTS _hosted_migrations(version INTEGER PRIMARY KEY, \
				 checksum TEXT NOT NULL)",
			)?;
			let current: Option<(u32, String)> = tx
				.query_row(
					"SELECT version,checksum FROM _hosted_migrations ORDER BY version DESC LIMIT 1",
					[],
					|r| Ok((r.get(0)?, r.get(1)?)),
				)
				.optional()?;
			let up = include_str!("migrations/001.up.sql");
			let checksum = digest(up);
			if let Some((version, old)) = current {
				ensure!(
					version == 1 && old == checksum,
					"unknown or modified hosted migration"
				);
			} else {
				tx.execute_batch(up)?;
				tx.execute("INSERT INTO _hosted_migrations VALUES(1,?1)", [checksum])?;
			}
			tx.commit()?;
		}
		Ok(Self {
			meta,
			root: root.into(),
		})
	}

	pub fn create(&self, id: &str, customer: &str) -> Result<String> {
		ensure!(
			control::identifier(id)
				&& customer.starts_with("cus_")
				&& control::identifier(customer),
			"invalid account/customer"
		);
		let key = format!("{id}.{}", control::random_key()?);
		self.meta.db()?.execute(
			"INSERT INTO hosted_accounts(id,key_hash,customer,status,plan) \
			 VALUES(?1,?2,?3,'inactive','control')",
			params![id, digest(&key), customer],
		)?;
		Ok(key)
	}

	pub fn rotate(&self, id: &str) -> Result<String> {
		let key = format!("{id}.{}", control::random_key()?);
		ensure!(
			self.meta.db()?.execute(
				"UPDATE hosted_accounts SET key_hash=?1 WHERE id=?2",
				params![digest(&key), id]
			)? == 1,
			"unknown account"
		);
		Ok(key)
	}

	pub(crate) fn account(&self, key: &str) -> Result<String> {
		ensure!(key.len() <= 256, "unauthorized");
		self.meta
			.db()?
			.query_row(
				"SELECT id FROM hosted_accounts WHERE key_hash=?1",
				[digest(key)],
				|r| r.get(0),
			)
			.optional()?
			.context("unauthorized")
	}

	pub fn view(&self, account: &str) -> Result<AccountView> {
		let db = self.meta.db()?;
		let (status, plan) = db.query_row(
			"SELECT status,plan FROM hosted_accounts WHERE id=?1",
			[account],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		drop(db);
		Ok(AccountView {
			id: account.into(),
			status,
			plan,
			installations: self.installations(account)?,
		})
	}

	pub(crate) fn entitled(&self, account: &str) -> Result<()> {
		let db = self.meta.db()?;
		let allowed: bool = db.query_row(
			"SELECT status IN ('active','trialing') AND valid_until>?2 FROM hosted_accounts WHERE \
			 id=?1",
			params![account, sql_now()],
			|r| r.get(0),
		)?;
		ensure!(allowed, "subscription_required");
		Ok(())
	}

	pub(crate) fn customer(&self, account: &str) -> Result<String> {
		Ok(self.meta.db()?.query_row(
			"SELECT customer FROM hosted_accounts WHERE id=?1",
			[account],
			|r| r.get(0),
		)?)
	}

	pub fn enable_managed(&self, account: &str, enabled: bool) -> Result<()> {
		ensure!(
			self.meta.db()?.execute(
				"UPDATE hosted_accounts SET managed_enabled=?1 WHERE id=?2",
				params![enabled, account]
			)? == 1,
			"unknown account"
		);
		Ok(())
	}

	pub(crate) fn managed_enabled(&self, account: &str) -> Result<bool> {
		Ok(self.meta.db()?.query_row(
			"SELECT managed_enabled FROM hosted_accounts WHERE id=?1",
			[account],
			|r| r.get(0),
		)?)
	}

	pub(crate) fn install(
		&self,
		account: &str,
		project: &str,
		environment: &str,
		ownership: Ownership,
	) -> Result<Installation> {
		ensure!(
			!environment.is_empty()
				&& environment.len() <= 128
				&& environment.bytes().all(|b| (32..127).contains(&b)),
			"invalid environment"
		);
		self.entitled(account)?;
		let mut db = self.meta.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let (plan, status, count): (String, String, u32) = tx.query_row(
			"SELECT plan,status,(SELECT count(*) FROM hosted_installations WHERE account=?1) FROM \
			 hosted_accounts WHERE id=?1",
			[account],
			|r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
		)?;
		ensure!(
			matches!(status.as_str(), "active" | "trialing")
				&& count < if plan == "team" { 20 } else { 1 },
			"subscription or installation limit"
		);
		let id = control::random_key()?;
		tx.execute(
			"INSERT INTO hosted_installations(id,account,project,environment,ownership) \
			 VALUES(?1,?2,?3,?4,?5)",
			params![
				id,
				account,
				project,
				environment,
				serde_json::to_string(&ownership)?
			],
		)?;
		tx.commit()?;
		Ok(Installation {
			id,
			project: project.into(),
			environment: environment.into(),
			ownership,
			last_seen: 0,
		})
	}

	pub fn installations(&self, account: &str) -> Result<Vec<Installation>> {
		let db = self.meta.db()?;
		let mut q = db.prepare(
			"SELECT id,project,environment,ownership,last_seen FROM hosted_installations WHERE \
			 account=?1 ORDER BY project",
		)?;
		let rows = q
			.query_map([account], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, String>(2)?,
					r.get::<_, String>(3)?,
					r.get::<_, i64>(4)?,
				))
			})?
			.collect::<rusqlite::Result<Vec<_>>>()?;
		rows.into_iter()
			.map(|(id, project, environment, ownership, last_seen)| {
				Ok(Installation {
					id,
					project,
					environment,
					ownership: serde_json::from_str(&ownership)?,
					last_seen: u64::try_from(last_seen).unwrap_or(0),
				})
			})
			.collect()
	}

	pub(crate) fn installation(&self, account: &str, project: &str) -> Result<Installation> {
		self.installations(account)?
			.into_iter()
			.find(|i| i.project == project)
			.context("installation_missing")
	}

	/// A ten-minute, single-use pairing grant; its replacement worker key is
	/// hashed.
	pub fn pairing(&self, account: &str, project: &str) -> Result<String> {
		self.installation(account, project)?;
		let key = format!("{account}.{}", control::random_key()?);
		self.meta.db()?.execute(
			"UPDATE hosted_installations SET pair_hash=?1,pair_until=?2 WHERE account=?3 AND \
			 project=?4",
			params![digest(&key), sql_now() + 600, account, project],
		)?;
		Ok(key)
	}

	pub(crate) fn redeem(&self, code: &str) -> Result<(String, Installation, String)> {
		ensure!(code.len() <= 256, "invalid pairing code");
		let mut db = self.meta.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let (account, project): (String, String) = tx.query_row(
			"SELECT account,project FROM hosted_installations WHERE pair_hash=?1 AND pair_until>?2",
			params![digest(code), sql_now()],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		let key = format!("{account}.{}", control::random_key()?);
		tx.execute(
			"UPDATE hosted_installations SET worker_hash=?1,pair_hash=NULL,pair_until=0 WHERE \
			 account=?2 AND project=?3",
			params![digest(&key), account, project],
		)?;
		tx.commit()?;
		drop(db);
		let installation = self.installation(&account, &project)?;
		Ok((account, installation, key))
	}

	pub(crate) fn worker(&self, key: &str) -> Result<(String, Installation)> {
		ensure!(key.len() <= 256, "unauthorized");
		let db = self.meta.db()?;
		let (account, project): (String, String) = db.query_row(
			"SELECT account,project FROM hosted_installations WHERE worker_hash=?1",
			[digest(key)],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		drop(db);
		Ok((account.clone(), self.installation(&account, &project)?))
	}

	pub fn revoke_worker(&self, account: &str, project: &str) -> Result<()> {
		ensure!(
			self.meta.db()?.execute(
				"UPDATE hosted_installations SET worker_hash=NULL,pair_hash=NULL,pair_until=0 \
				 WHERE account=?1 AND project=?2",
				params![account, project]
			)? == 1,
			"installation_missing"
		);
		Ok(())
	}

	pub(crate) fn seen(&self, id: &str) -> Result<()> {
		self.meta.db()?.execute(
			"UPDATE hosted_installations SET last_seen=?1 WHERE id=?2",
			params![sql_now(), id],
		)?;
		Ok(())
	}
}
