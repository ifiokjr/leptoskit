//! The client hooks against a real server on SQLite.

#![cfg(all(feature = "live", feature = "sqlite"))]

mod live_hooks;

#[tokio::test]
async fn hooks_on_sqlite() {
	live_hooks::hooks(leptoskit_db::testing::sqlite().await).await;
}
