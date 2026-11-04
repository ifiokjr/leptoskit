//! Hosted account broker. Customer code executes only on dedicated workers.
mod api;
mod billing;
mod state;
mod store;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
pub use billing::Billing;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
pub use store::Accounts;

use crate::control::Action;
use crate::control::Operation;
use crate::control::Project;
use crate::control::Service;
use crate::control::Store;
use crate::control::{self};
pub use crate::hosted_protocol::*;

#[derive(Serialize, Deserialize)]
pub struct Job {
	pub operation: Operation,
	pub analytics_read_key: String,
	pub analytics_ingest_key: String,
}

#[derive(Clone)]
pub struct Hosted(Arc<Inner>);
struct Inner {
	accounts: Accounts,
	secret: String,
	origin: String,
	billing: Option<Billing>,
}
impl Hosted {
	/// State encryption material is supplied independently of account keys.
	pub fn open(
		data: &Path,
		secret: String,
		origin: &str,
		billing: Option<Billing>,
	) -> Result<Self> {
		ensure!(
			secret.len() >= 32 && secret.len() <= 256,
			"hosted key needs 32..=256 bytes"
		);
		// Reuse the self-hosted origin and key validation rules.
		let accounts = Accounts::open(data)?;
		Service::new(accounts.meta.clone(), &secret, origin)?;
		let host = Self(Arc::new(Inner {
			accounts,
			secret,
			origin: origin.trim_end_matches('/').into(),
			billing,
		}));
		let verifier = host.derived("hosted-key-check-v1");
		let db = host.accounts().meta.db()?;
		db.execute(
			"INSERT OR IGNORE INTO hosted_key_verifier VALUES(1,?1)",
			[&verifier],
		)?;
		let stored: String = db.query_row(
			"SELECT value FROM hosted_key_verifier WHERE id=1",
			[],
			|r| r.get(0),
		)?;
		ensure!(
			stored == verifier,
			"hosted encryption key does not match this database"
		);
		drop(db);
		Ok(host)
	}

	/// Reconcile only after stopping/revoking the old worker and inspecting
	/// provider state.
	pub fn interrupt(&self, account: &str, id: &str) -> Result<()> {
		let (store, _) = self.tenant(account)?;
		let operation = store.operation(id)?.context("operation missing")?;
		let installation = self.accounts().installation(account, &operation.project)?;
		let active: bool = self.accounts().meta.db()?.query_row(
			"SELECT worker_hash IS NOT NULL FROM hosted_installations WHERE id=?1",
			[installation.id],
			|r| r.get(0),
		)?;
		ensure!(!active, "revoke the old worker before reconciling");
		ensure!(
			store.db()?.execute(
				"UPDATE operations SET phase='interrupted',updated=?1,message='Operator \
				 reconciled interrupted dedicated worker' WHERE id=?2 AND phase='running'",
				rusqlite::params![sql_now(), id]
			)? == 1,
			"operation is not running"
		);
		Ok(())
	}

	/// Offline consistent copies of metadata and every tenant journal, into a
	/// new private directory.
	pub fn backup(&self, destination: &Path) -> Result<()> {
		ensure!(!destination.exists(), "backup destination exists");
		std::fs::create_dir(destination)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o700))?;
		}
		self.accounts()
			.meta
			.backup(&destination.join("accounts.sqlite3"))?;
		let db = self.accounts().meta.db()?;
		let mut q = db.prepare("SELECT id FROM hosted_accounts")?;
		let ids = q
			.query_map([], |r| r.get::<_, String>(0))?
			.collect::<rusqlite::Result<Vec<_>>>()?;
		drop(q);
		drop(db);
		for id in ids {
			let (store, _) = self.tenant(&id)?;
			std::fs::create_dir(destination.join(&id))?;
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt as _;
				std::fs::set_permissions(
					destination.join(&id),
					std::fs::Permissions::from_mode(0o700),
				)?;
			}
			store.backup(&destination.join(id).join("journal.sqlite3"))?;
		}
		Ok(())
	}

	/// Reverse only the hosted schema, after an offline backup of every
	/// journal.
	pub fn rollback_schema(&self, backup: &Path) -> Result<()> {
		self.backup(backup)?;
		let mut db = self.accounts().meta.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		tx.execute_batch(include_str!("migrations/001.down.sql"))?;
		tx.execute("DELETE FROM _hosted_migrations", [])?;
		tx.commit()?;
		Ok(())
	}

	/// Restore into a new directory; old workers/keys cannot revive from a
	/// backup.
	pub fn restore_new(source: &Path, destination: &Path, secret: String) -> Result<()> {
		ensure!(
			!destination.exists(),
			"restore needs a new destination directory"
		);
		let parent = destination
			.parent()
			.context("restore needs a parent directory")?;
		std::fs::create_dir_all(parent)?;
		let candidate = tempfile::tempdir_in(parent)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::set_permissions(candidate.path(), std::fs::Permissions::from_mode(0o700))?;
		}
		std::fs::copy(
			source.join("accounts.sqlite3"),
			candidate.path().join("accounts.sqlite3"),
		)?;
		let host = Self::open(candidate.path(), secret, "http://localhost:4701", None)?;
		let ids = {
			let db = host.accounts().meta.db()?;
			let mut q = db.prepare("SELECT id FROM hosted_accounts")?;
			q.query_map([], |r| r.get::<_, String>(0))?
				.collect::<rusqlite::Result<Vec<_>>>()?
		};
		for account in ids {
			ensure!(control::identifier(&account), "invalid restored account");
			let dir = candidate.path().join(&account);
			std::fs::create_dir(&dir)?;
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt as _;
				std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
			}
			std::fs::copy(
				source.join(&account).join("journal.sqlite3"),
				dir.join("journal.sqlite3"),
			)?;
			let (store, _) = host.tenant(&account)?;
			verify_store(&store)?;
			store.interrupt_running()?;
			store.db()?.execute("UPDATE tokens SET revoked=1", [])?;
			host.accounts().rotate(&account)?;
			for installation in host.accounts().installations(&account)? {
				host.state_read(&account, &installation.project)?;
				host.accounts()
					.revoke_worker(&account, &installation.project)?;
			}
		}
		verify_store(&host.accounts().meta)?;
		drop(host);
		std::fs::rename(candidate.path(), destination)?;
		Ok(())
	}

	pub fn accounts(&self) -> &Accounts {
		&self.0.accounts
	}

	pub(crate) fn tenant(&self, account: &str) -> Result<(Store, Service)> {
		ensure!(control::identifier(account), "invalid account");
		let store = Store::open(&self.0.accounts.root.join(account).join("journal.sqlite3"))?;
		let owner = self.derived(&format!("tenant-owner:{account}"));
		let service = Service::new(store.clone(), &owner, &self.0.origin)?;
		Ok((store, service))
	}

	pub(crate) fn derived(&self, label: &str) -> String {
		use hmac::Mac as _;
		let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(self.0.secret.as_bytes())
			.expect("HMAC accepts any key size");
		mac.update(label.as_bytes());
		hex(&mac.finalize().into_bytes())
	}

	pub(crate) fn authorize(&self, key: &str) -> Result<String> {
		self.0.accounts.account(key)
	}

	pub fn router(&self) -> axum::Router {
		api::router(self.clone())
	}

	pub fn register(
		&self,
		account: &str,
		project: &str,
		environment: &str,
		ownership: Ownership,
	) -> Result<Installation> {
		ensure!(control::identifier(project), "invalid project");
		let (store, _) = self.tenant(account)?;
		let installation = self
			.0
			.accounts
			.install(account, project, environment, ownership)?;
		store.register(&Project {
			id: project.into(),
			environment: environment.into(),
			root: Path::new("/hosted-no-execution").join(project),
			revision: 0,
		})?;
		Ok(installation)
	}

	pub(crate) fn check_job_policy(
		&self,
		account: &str,
		project: &str,
		action: &Action,
	) -> Result<()> {
		let installation = self.0.accounts.installation(account, project)?;
		if matches!(installation.ownership, Ownership::LeptoskitDigitalocean) {
			// Paid resources are quoted/configured on the dedicated operator
			// worker.
			ensure!(
				self.0.accounts.managed_enabled(account)?,
				"managed capacity is not enabled for this account"
			);
		}
		if !matches!(
			action,
			Action::Status | Action::Backup | Action::Rollback { .. }
		) {
			self.0.accounts.entitled(account)?;
		}
		Ok(())
	}
}
fn digest(value: &str) -> String {
	format!("{:x}", sha2::Sha256::digest(value.as_bytes()))
}

fn sql_now() -> i64 {
	i64::try_from(control::now()).unwrap_or(i64::MAX)
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write as _;
	let mut out = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		write!(&mut out, "{byte:02x}").expect("formatting to String");
	}
	out
}

fn verify_store(store: &Store) -> Result<()> {
	let db = store.db()?;
	let status: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
	ensure!(status == "ok", "backup integrity check failed");
	let mut query = db.prepare("PRAGMA foreign_key_check")?;
	ensure!(
		query.query([])?.next()?.is_none(),
		"backup foreign key check failed"
	);
	Ok(())
}
