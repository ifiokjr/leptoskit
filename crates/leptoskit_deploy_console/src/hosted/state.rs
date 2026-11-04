use aes_gcm_siv::Aes256GcmSiv;
use aes_gcm_siv::KeyInit;
use aes_gcm_siv::Nonce;
use aes_gcm_siv::aead::Aead;
use aes_gcm_siv::aead::Payload;
use rusqlite::OptionalExtension as _;
use rusqlite::params;

use super::*;

impl Hosted {
	fn cipher(&self) -> Aes256GcmSiv {
		Aes256GcmSiv::new_from_slice(&sha2::Sha256::digest(
			self.derived("state-encryption-v1").as_bytes(),
		))
		.expect("32-byte encryption key")
	}

	pub fn state_read(&self, account: &str, project: &str) -> Result<Option<Vec<u8>>> {
		self.accounts().installation(account, project)?;
		let db = self.accounts().meta.db()?;
		let row: Option<(i64, Vec<u8>)> = db
			.query_row(
				"SELECT version,body FROM hosted_states WHERE account=?1 AND project=?2 ORDER BY \
				 version DESC LIMIT 1",
				params![account, project],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()?;
		row.map(|(version, body)| {
			ensure!(body.len() >= 12, "invalid state envelope");
			self.cipher()
				.decrypt(
					Nonce::from_slice(&body[..12]),
					Payload {
						msg: &body[12..],
						aad: format!("{account}/{project}/{version}").as_bytes(),
					},
				)
				.map_err(|_| anyhow::anyhow!("state decryption failed"))
		})
		.transpose()
	}

	pub fn state_lock(
		&self,
		account: &str,
		project: &str,
		body: &serde_json::Value,
		unlock: bool,
	) -> Result<Option<serde_json::Value>> {
		self.accounts().installation(account, project)?;
		let id = body["ID"].as_str().context("lock needs ID")?;
		ensure!(
			id.len() <= 256 && serde_json::to_vec(body)?.len() <= 16384,
			"invalid lock"
		);
		let mut db = self.accounts().meta.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let old: Option<(String, String)> = tx
			.query_row(
				"SELECT id,body FROM hosted_locks WHERE account=?1 AND project=?2",
				params![account, project],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()?;
		if let Some((old_id, old_body)) = &old
			&& old_id != id
		{
			return Ok(Some(serde_json::from_str(old_body)?));
		}
		if unlock {
			ensure!(old.is_some(), "lock_missing");
			tx.execute(
				"DELETE FROM hosted_locks WHERE account=?1 AND project=?2",
				params![account, project],
			)?;
		} else {
			tx.execute(
				"INSERT OR IGNORE INTO hosted_locks VALUES(?1,?2,?3,?4)",
				params![account, project, id, serde_json::to_string(body)?],
			)?;
		}
		tx.commit()?;
		Ok(None)
	}

	/// Never accepts unlocked writes, including the first state. Retains 20
	/// encrypted versions.
	pub fn state_write(&self, account: &str, project: &str, lock: &str, body: &[u8]) -> Result<()> {
		self.accounts().installation(account, project)?;
		ensure!(body.len() <= 8 * 1024 * 1024, "state too large");
		let value: serde_json::Value = serde_json::from_slice(body)?;
		let serial = value["serial"].as_u64().context("state needs serial")?;
		let lineage = value["lineage"].as_str().context("state needs lineage")?;
		let mut db = self.accounts().meta.db()?;
		let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
		let holder: String = tx.query_row(
			"SELECT id FROM hosted_locks WHERE account=?1 AND project=?2",
			params![account, project],
			|r| r.get(0),
		)?;
		ensure!(holder == lock, "state_lock_conflict");
		let old: Option<(i64, Vec<u8>)> = tx
			.query_row(
				"SELECT version,body FROM hosted_states WHERE account=?1 AND project=?2 ORDER BY \
				 version DESC LIMIT 1",
				params![account, project],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()?;
		let mut version = 1;
		if let Some((previous, envelope)) = old {
			ensure!(envelope.len() >= 12, "invalid state envelope");
			let clear = self
				.cipher()
				.decrypt(
					Nonce::from_slice(&envelope[..12]),
					Payload {
						msg: &envelope[12..],
						aad: format!("{account}/{project}/{previous}").as_bytes(),
					},
				)
				.map_err(|_| anyhow::anyhow!("state decryption failed"))?;
			let old: serde_json::Value = serde_json::from_slice(&clear)?;
			ensure!(
				old["lineage"].as_str() == Some(lineage)
					&& old["serial"].as_u64().is_some_and(|s| serial >= s),
				"state lineage or serial conflict"
			);
			if old["serial"].as_u64() == Some(serial) {
				ensure!(clear == body, "state serial reused with different content");
				return Ok(());
			}
			version = previous + 1;
		}
		let mut nonce = [0u8; 12];
		getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("randomness unavailable"))?;
		let encrypted = self
			.cipher()
			.encrypt(
				Nonce::from_slice(&nonce),
				Payload {
					msg: body,
					aad: format!("{account}/{project}/{version}").as_bytes(),
				},
			)
			.map_err(|_| anyhow::anyhow!("state encryption failed"))?;
		let mut envelope = nonce.to_vec();
		envelope.extend(encrypted);
		tx.execute(
			"INSERT INTO hosted_states VALUES(?1,?2,?3,?4,?5)",
			params![account, project, version, envelope, sql_now()],
		)?;
		tx.execute(
			"DELETE FROM hosted_states WHERE account=?1 AND project=?2 AND version<=?3",
			params![account, project, version.saturating_sub(20)],
		)?;
		tx.commit()?;
		Ok(())
	}
}
