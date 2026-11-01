//! Upgrade the databases shipped before live session guards: existing
//! credentials survive, and subsequent revocations enter the change log.

#![cfg(debug_assertions)]

#[path = "../../../../crates/leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::time::Duration;

use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::RequestInfo;
use leptoskit_db::Db;
use leptoskit_db::Migration;
use leptoskit_db::live::LogEntry;
use postgres::PostgresCluster;

async fn upgrade(db: Db, migrations: &[Migration]) {
	assert!(migrations.len() >= 2);
	db.migrate(&migrations[..1]).await.unwrap();
	let auth = Auth::builder(&db, Keys::single("upgrade", [9; 32]))
		.mailer(Mailbox::new())
		.dev_sign_in()
		.build()
		.unwrap();
	let sign_in = auth
		.dev_sign_in("ada@upgrade.example", &RequestInfo::default())
		.await
		.unwrap();
	let account = sign_in.session.account_id;
	let applied = db.migrate(migrations).await.unwrap();
	assert_eq!(applied.len(), migrations.len() - 1);
	assert_eq!(
		auth.resolve(&sign_in.token).await.unwrap().unwrap().account,
		account
	);
	let mut log = db
		.system("test: upgrade capture")
		.change_log(None)
		.await
		.unwrap();
	auth.sign_out(&sign_in.token, &RequestInfo::default())
		.await
		.unwrap();
	let LogEntry::Commit(commit) = tokio::time::timeout(Duration::from_secs(5), log.next())
		.await
		.unwrap()
		.unwrap()
	else {
		panic!("the upgraded session write must be captured");
	};
	let session = commit
		.changes
		.iter()
		.find(|change| {
			change.table.namespace.as_deref() == Some("auth") && change.table.name == "sessions"
		})
		.expect("session capture is installed");
	for image in session.old.iter().chain(session.new.iter()) {
		assert!(!image.contains_key("token_hash"));
		assert!(!image.contains_key("refresh_hash"));
		assert!(!image.contains_key("previous_refresh_hash"));
	}
}

#[tokio::test]
async fn sqlite_keeps_existing_sessions_when_enabling_live_auth() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	upgrade(db, huddle_db::MIGRATIONS).await;
}

#[tokio::test]
async fn postgres_keeps_existing_sessions_when_enabling_live_auth() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	upgrade(db, huddle_db::POSTGRES_MIGRATIONS).await;
}
