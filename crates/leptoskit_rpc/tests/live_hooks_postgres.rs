//! The client hooks against a real server on Postgres.

#![cfg(all(feature = "live", feature = "postgres"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

mod live_hooks;

#[tokio::test]
async fn hooks_on_postgres() {
	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let db = leptoskit_db::Db::connect(&cluster.database())
		.await
		.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	live_hooks::hooks(db).await;
}
