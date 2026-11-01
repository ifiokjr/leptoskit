/// These checks run through the same guest ABI as the scaffold buttons.
async fn optional_workers() -> Result<(), leptoskit_bridge::Error> {
	sqlite_worker().await?;
	store_worker().await?;
	crypto_worker().await?;
	logging_worker().await?;
	image_worker().await?;
	Ok(())
}

/// A 2×1 RGBA PNG: one red pixel, then one blue pixel.
const PNG_2X1: &[u8] = &[
	137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
	0, 0, 244, 34, 127, 138, 0, 0, 0, 14, 73, 68, 65, 84, 120, 218, 99, 248, 207, 192, 0, 66, 255,
	1, 15, 249, 3, 253, 152, 121, 215, 0, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

async fn image_worker() -> Result<(), leptoskit_bridge::Error> {
	use leptoskit_bridge::Error;
	use leptoskit_plugin_image_manipulator::Format;
	use leptoskit_plugin_image_manipulator::ImageInfo;
	use leptoskit_plugin_image_manipulator::Operation;
	use leptoskit_plugin_image_manipulator::Output;
	use leptoskit_plugin_image_manipulator::Rotation;
	use leptoskit_plugin_image_manipulator::guest::ImageManipulator;
	let images = ImageManipulator::new()?;
	let source = images.inspect(PNG_2X1).await?;

	if source
		!= (ImageInfo {
			format: Format::Png,
			width: 2,
			height: 1,
		}) {
		return Err(Error::Native);
	}
	let rotated = images
		.manipulate(
			PNG_2X1,
			&[Operation::Rotate(Rotation::Clockwise90)],
			Output::Jpeg {
				quality: 80,
				background: [255; 3],
			},
		)
		.await?;

	if rotated.info
		!= (ImageInfo {
			format: Format::Jpeg,
			width: 1,
			height: 2,
		}) {
		return Err(Error::Native);
	}
	let outside = images
		.manipulate(
			PNG_2X1,
			&[Operation::Crop {
				x: 1,
				y: 0,
				width: 2,
				height: 1,
			}],
			Output::Png,
		)
		.await;

	if outside != Err(Error::InvalidArgument) {
		return Err(Error::Native);
	}

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
