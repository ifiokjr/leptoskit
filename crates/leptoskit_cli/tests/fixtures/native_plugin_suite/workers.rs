/// These checks run through the same guest ABI as the scaffold buttons.
async fn optional_workers() -> Result<(), leptoskit_bridge::Error> {
	sqlite_worker().await?;
	store_worker().await?;
	crypto_worker().await?;
	logging_worker().await?;
	Ok(())
}

async fn crypto_worker() -> Result<(), leptoskit_bridge::Error> {
	let crypto = leptoskit_plugin_crypto::guest::Crypto::new()?;
	let first = crypto.random_bytes(32).await?;
	let second = crypto.random_bytes(32).await?;
	let uuid = crypto.uuid_v4().await?;
	let bytes = uuid.as_bytes();
	if first.len() != 32
		|| second.len() != 32
		|| first == second
		|| bytes[6] >> 4 != 4
		|| bytes[8] & 0xc0 != 0x80
		|| !crypto.random_bytes(0).await?.is_empty()
	{
		return Err(leptoskit_bridge::Error::Native);
	}
	Ok(())
}

async fn logging_worker() -> Result<(), leptoskit_bridge::Error> {
	use leptoskit_plugin_logging::Level;
	use leptoskit_plugin_logging::Record;
	use leptoskit_plugin_logging::guest::Logger;
	let logger = Logger::new()?;
	let mut record = Record {
		level: Level::Debug,
		target: "integration.worker".into(),
		message: "Nonsecret Android plugin fixture".into(),
		fields: [("fixture".into(), "native_plugin_suite".into())].into(),
	};
	if logger.write(&record).await? {
		return Err(leptoskit_bridge::Error::Native);
	}
	record.level = Level::Info;
	if !logger.write(&record).await? {
		return Err(leptoskit_bridge::Error::Native);
	}
	logger.flush().await?;
	Ok(())
}

async fn sqlite_worker() -> Result<(), leptoskit_bridge::Error> {
	use leptoskit_plugin_sqlite::Statement;
	use leptoskit_plugin_sqlite::Value;
	use leptoskit_plugin_sqlite::guest::Sqlite;
	let database = Sqlite::new()?;
	database
		.execute(&Statement::new(
			"CREATE TABLE IF NOT EXISTS plugin_smoke (id INTEGER PRIMARY KEY, message TEXT, value \
			 BLOB)",
			vec![],
		))
		.await?;
	database
		.transaction(&[
			Statement::new("DELETE FROM plugin_smoke", vec![]),
			Statement::new(
				"INSERT INTO plugin_smoke (id, message, value) VALUES (?, ?, ?)",
				vec![
					Value::Integer(7),
					Value::Text("SQLite worker ready".into()),
					Value::Blob(vec![0, 127, 255]),
				],
			),
		])
		.await?;
	let failed = database
		.transaction(&[
			Statement::new(
				"UPDATE plugin_smoke SET message = ? WHERE id = ?",
				vec![Value::Text("must roll back".into()), Value::Integer(7)],
			),
			Statement::new(
				"INSERT INTO plugin_smoke (id) VALUES (?)",
				vec![Value::Integer(7)],
			),
		])
		.await;
	if failed != Err(leptoskit_bridge::Error::Native) {
		return Err(leptoskit_bridge::Error::Native);
	}
	database.close().await?;
	let result = database
		.query(&Statement::new(
			"SELECT id, message, value FROM plugin_smoke WHERE id = ?",
			vec![Value::Integer(7)],
		))
		.await?;
	database.close().await?;
	if result.rows
		!= [vec![
			Value::Integer(7),
			Value::Text("SQLite worker ready".into()),
			Value::Blob(vec![0, 127, 255]),
		]] {
		return Err(leptoskit_bridge::Error::Native);
	}
	Ok(())
}

async fn store_worker() -> Result<(), leptoskit_bridge::Error> {
	use leptoskit_plugin_store::Value;
	use leptoskit_plugin_store::guest::Store;
	let store = Store::new()?;
	let key = "integration.worker";
	let expected = serde_json::json!({"ready": true, "value": 42});
	store.set(key, expected.clone()).await?;
	store.save().await?;
	store.set(key, Value::Null).await?;
	if store.get(key).await? != Some(Value::Null) {
		return Err(leptoskit_bridge::Error::Native);
	}
	store.load().await?;
	if store.get(key).await? != Some(expected) {
		return Err(leptoskit_bridge::Error::Native);
	}
	store.remove(key).await?;
	store.save().await?;
	store.load().await?;
	if store.get(key).await?.is_some() {
		return Err(leptoskit_bridge::Error::Native);
	}
	Ok(())
}
