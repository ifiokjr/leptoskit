//! `huddle`: the web client, the API, the live socket, sign-in, and the job
//! workers, in one process on one SQLite file.
//!
//! | Variable                          | Default                    | What it does                                   |
//! | --------------------------------- | -------------------------- | ---------------------------------------------- |
//! | `DATABASE_URL`                    | `sqlite://huddle.sqlite3`  | The database                                   |
//! | `LEPTOS_SITE_ADDR`                | `127.0.0.1:3000`           | Where to listen                                |
//! | `LEPTOS_SITE_ROOT`                | `dist`                     | The built web client                           |
//! | `AUTH_ENCRYPTION_KEYS`            | a development key in debug | Sign-in's keys; required in release            |
//! | `HUDDLE_ORIGIN`                   | `[auth] origin`            | The site's origin, for cookies and the socket  |
//! | `HUDDLE_DEMO_MAIL`                | on in debug                | `1` keeps email at `/_leptoskit/mail`          |
//! | `HUDDLE_SETTLEMENT_DELAY_MS`      | `2000`                     | How long the simulated settlement takes        |
//! | `HUDDLE_SETTLEMENT_FAILURE_RATE`  | `0.2`                      | How often it declines a tip, from 0 to 1       |

use std::net::SocketAddr;
use std::time::Duration;

use huddle_server::Options;
use huddle_server::sign_in;
use leptoskit::server::GracefulShutdown;
use leptoskit::server::Readiness;
use leptoskit::server::tokio;
use leptoskit_rpc::server::Jobs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	if !cfg!(debug_assertions) && std::env::var("HUDDLE_DEMO_MAIL").as_deref() != Ok("1") {
		return Err(
			"Huddle keeps every email in its demo mailbox, which anyone can read at \
			 /_leptoskit/mail. Set HUDDLE_DEMO_MAIL=1 to run it that way, as the demo does, or \
			 give huddle_api's services a mailer that sends."
				.into(),
		);
	}

	let readiness = Readiness::new();
	let migrated = readiness.check("migrations");
	let address: SocketAddr = std::env::var("LEPTOS_SITE_ADDR")
		.unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
		.parse()?;
	let site_root = std::env::var("LEPTOS_SITE_ROOT").unwrap_or_else(|_| "dist".to_owned());

	let url =
		std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://huddle.sqlite3".to_owned());
	let db = leptoskit::db::Db::connect(&url).await?;

	for name in db.migrate(huddle_db::MIGRATIONS).await? {
		println!("applied migration {name}");
	}

	migrated.pass();

	let config = sign_in::config()?;
	let mut options = Options::new(&db);
	options.origin = std::env::var("HUDDLE_ORIGIN")
		.ok()
		.or_else(|| config.auth.as_ref().and_then(|auth| auth.origin.clone()));
	options.site_root = Some(site_root.into());
	options.jobs = Jobs::from_config(db.clone(), &config.rpc.jobs);
	options.settlement.failure_rate = 0.2;

	if let Ok(delay) = std::env::var("HUDDLE_SETTLEMENT_DELAY_MS") {
		options.settlement.delay = Duration::from_millis(delay.parse()?);
	}

	if let Ok(rate) = std::env::var("HUDDLE_SETTLEMENT_FAILURE_RATE") {
		options.settlement.failure_rate = rate.parse::<f64>()?.clamp(0.0, 1.0);
	}

	let huddle = huddle_server::start(&db, sign_in::builder(&db)?, options)?;
	let app = huddle
		.router
		.merge(readiness.routes())
		// The native apps' association files, for their passkeys.
		.merge(sign_in::associations()?);

	let listener = tokio::net::TcpListener::bind(address).await?;
	println!("listening on http://{address}");

	// On Ctrl-C or SIGTERM: fail /readyz, tell every live socket to
	// reconnect elsewhere (with jitter, so they don't all come back at
	// once), finish the requests in flight, then stop the workers, which
	// finish or release the jobs they hold.
	let live = huddle.live.clone();
	let shutdown = GracefulShutdown::new()
		.readiness(&readiness)
		.signal(async move {
			leptoskit::server::shutdown_signal().await;
			live.drain();
		});
	let served = leptoskit::server::serve_listener_with(listener, app, shutdown).await;
	huddle.jobs.shutdown().await;
	served?;

	Ok(())
}
