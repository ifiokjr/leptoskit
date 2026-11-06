//! The analytics service: it opens and migrates its own database, then
//! serves its operations and live connection under
//! `/_leptoskit/services/analytics`.
//!
//! `lk dev` and `lk serve` start it with everything it reads from the
//! environment: its address, its database, its token, and the app's origin.

use analytics_service::MIGRATIONS;
#[cfg(feature = "postgres")]
use analytics_service::POSTGRES_MIGRATIONS;
use analytics_service::SERVICE;
use leptoskit_axum::axum::Router;
use leptoskit_axum::service::ServiceEnv;
use leptoskit_db::Db;
use leptoskit_db::PoolOptions;

// Links the operations library, so its operations are registered, and
// serves what `run` returns with `/healthz` and `/readyz`.
leptoskit_axum::service::main!(analytics_service, run);

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn run(env: ServiceEnv) -> Result<Router, Error> {
	// The operations are addressed to `analytics`: under any other name, every
	// call the app makes would go somewhere this service isn't.
	env.expect_name(SERVICE)?;

	let url = env
		.database_url()
		.ok_or("the analytics service needs a database: set `database = true`")?;
	let db = match env.database_pool() {
		Some(max_connections) => Db::connect_with(url, PoolOptions { max_connections }).await?,
		None => Db::connect(url).await?,
	};

	// The migrations for the engine `url` names.
	#[cfg(feature = "postgres")]
	let migrations = if url.starts_with("postgres") {
		POSTGRES_MIGRATIONS
	} else {
		MIGRATIONS
	};
	#[cfg(not(feature = "postgres"))]
	let migrations = if url.starts_with("postgres") {
		return Err(
			"this build has no Postgres support: add `postgres` to the service's features".into(),
		);
	} else {
		MIGRATIONS
	};

	for name in db.migrate(migrations).await? {
		println!("applied migration {name}");
	}

	let (router, live) = analytics_service::routes(db, env.public_origin());
	live.start().await?;
	Ok(router)
}
