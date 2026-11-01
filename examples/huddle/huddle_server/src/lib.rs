//! Huddle's server, as a library: [`start`] builds everything the binary
//! serves, so the tests can start the same server in process.
//!
//! One router carries it all:
//!
//! | Route              | What it is                                                                 |
//! | ------------------ | -------------------------------------------------------------------------- |
//! | `POST /api/<name>` | Every query, mutation, and action in `huddle_api`, as the caller           |
//! | `GET /live`        | The live socket: subscriptions, the typing room, and presence              |
//! | `/auth/…`          | Sign-in, from `leptoskit_auth`, with dev sign-in in debug builds           |
//! | `/_leptoskit/mail` | The demo mailbox: sign-in codes and invitations, when mail stays in-app    |
//! | everything else    | The web client from `dist/`, built by `leptoskit build`                    |

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use huddle_api::Settlement;
use huddle_api::services::Services;
use leptoskit::db::Db;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::response::Html;
use leptoskit::server::axum::routing::get;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthBuilder;
use leptoskit_auth::AuthError;
use leptoskit_auth::Mailbox;
use leptoskit_rpc::server::Jobs;
use leptoskit_rpc::server::JobsHandle;
use leptoskit_rpc::server::Live;

pub mod sign_in;

/// How to start Huddle.
pub struct Options {
	/// The site's origin, such as `https://huddle-demo.fly.dev`. The live
	/// socket refuses pages from anywhere else; native apps, which send no
	/// `Origin`, sign in with a token instead.
	pub origin: Option<String>,
	/// Where the web client was built: `dist/`, unless `LEPTOS_SITE_ROOT`
	/// says otherwise. `None` serves no files, as the tests do.
	pub site_root: Option<PathBuf>,
	/// How the simulated settlement service behaves.
	pub settlement: Settlement,
	/// How long an invitation takes to "send".
	pub mail_delay: Duration,
	/// The job workers: how many, and each queue's limit.
	pub jobs: Jobs,
}

impl Options {
	/// The defaults for `db`: no origin check, no files, a settlement that
	/// takes two seconds and never fails, and four workers.
	pub fn new(db: &Db) -> Self {
		Self {
			origin: None,
			site_root: None,
			settlement: Settlement::default(),
			mail_delay: Duration::from_millis(1_500),
			jobs: Jobs::new(db.clone())
				.queue("mail", 2)
				.queue("settlement", 2),
		}
	}
}

impl std::fmt::Debug for Options {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Options")
			.field("origin", &self.origin)
			.field("site_root", &self.site_root)
			.field("settlement", &self.settlement)
			.field("mail_delay", &self.mail_delay)
			.finish_non_exhaustive()
	}
}

/// A running Huddle: its router, and the parts the binary shuts down.
pub struct Huddle {
	/// Everything Huddle serves.
	pub router: Router,
	/// The live socket's server, to drain on shutdown.
	pub live: Live,
	/// The job workers, to stop on shutdown.
	pub jobs: JobsHandle,
	/// Sign-in.
	pub auth: Auth,
	/// Where every email goes: sign-in codes and invitations.
	pub mailbox: Mailbox,
}

impl std::fmt::Debug for Huddle {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Huddle")
			.field("live", &self.live)
			.finish_non_exhaustive()
	}
}

/// Builds Huddle on a migrated `db`, with sign-in from `auth`, and starts
/// its job workers.
///
/// Every email, the sign-in codes and the invitations alike, goes to an
/// in-app [`Mailbox`] shown at `/_leptoskit/mail`. That's the development
/// mailbox, and what the public demo uses so anyone can sign in; a real
/// deployment gives `huddle_api`'s services a mailer that sends.
///
/// # Errors
///
/// When sign-in can't be built from `auth`.
pub fn start(db: &Db, auth: AuthBuilder, options: Options) -> Result<Huddle, AuthError> {
	let mailbox = Mailbox::new();
	let auth = auth
		.app_name("Huddle")
		.mailer(mailbox.clone())
		.on_sign_up::<huddle_db::User>();
	#[cfg(debug_assertions)]
	let auth = auth.dev_sign_in();
	let auth = auth.build()?;

	huddle_api::services::install(Services {
		mail_delay: options.mail_delay,
		settlement: options.settlement,
		..Services::new(auth.clone(), mailbox.clone())
	});

	let rpc = leptoskit_rpc::server::Server::new(db.clone()).identify(auth.clone());
	let live = match &options.origin {
		Some(origin) => rpc.live().origin(origin),
		None => rpc.live(),
	};
	let jobs = options.jobs.start();
	let shown = mailbox.clone();
	let mut router = Router::new()
		.merge(rpc.router())
		.merge(live.router())
		.merge(auth.router())
		.route(
			"/_leptoskit/mail",
			get(move || {
				let mailbox = shown.clone();
				async move { Html(mail_page(&mailbox)) }
			}),
		);

	if let Some(root) = &options.site_root {
		router = router.fallback_service(leptoskit::server::static_files(root));
	}

	// Last, so it wraps every route: it finds each request's session and
	// gives handlers a database that acts for whoever is signed in.
	let router = router.layer(auth.layer());

	Ok(Huddle {
		router,
		live,
		jobs,
		auth,
		mailbox,
	})
}

/// Every email so far, newest first, as a page.
fn mail_page(mailbox: &Mailbox) -> String {
	let mut page = String::from(
		"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Huddle \
		 mail</title><style>body{font:16px system-ui;max-width:42rem;margin:2rem auto;padding:0 \
		 1rem}article{border:1px solid #ccc;border-radius:8px;padding:1rem;margin:1rem \
		 0}h2{font-size:1rem;margin:0 0 .5rem}p{margin:.25rem \
		 0;white-space:pre-wrap}</style></head><body><h1>Huddle mail</h1><p>Every email Huddle \
		 sent, newest first. Nothing here left this server.</p>",
	);

	for mail in mailbox.messages().iter().rev() {
		let _ = write!(
			page,
			"<article><h2>{}</h2><p>To {}</p><p>{}</p></article>",
			escape(&mail.subject),
			escape(&mail.to),
			escape(&mail.text)
		);
	}

	page.push_str("</body></html>");
	page
}

fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}
