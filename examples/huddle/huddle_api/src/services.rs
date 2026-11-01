//! What the server's operations reach outside the database: sign-in, email,
//! the summarizer, and the simulated settlement service. The server installs
//! them once when it starts.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::time::Duration;

use leptoskit_auth::Auth;
use leptoskit_auth::Mailer;

use crate::summary::LocalSummarizer;
use crate::summary::Summarizer;
use crate::tips::Settlement;

/// Everything outside the database.
#[derive(Clone)]
pub struct Services {
	/// Sign-in, for signing people out everywhere.
	pub auth: Auth,
	/// Where invitations go. In development, the same mailbox as the
	/// sign-in codes.
	pub mailer: Arc<dyn Mailer>,
	/// How long an invitation takes to send, so *Sending…* is there long
	/// enough to see.
	pub mail_delay: Duration,
	/// What "Summarize this channel" asks.
	pub summarizer: Arc<dyn Summarizer>,
	/// Where tips settle: simulated, in this process.
	pub settlement: Settlement,
}

impl Services {
	/// The services for `auth` and `mailer`, with the local summarizer and a
	/// settlement service that takes two seconds and never fails.
	pub fn new(auth: Auth, mailer: impl Mailer) -> Self {
		Self {
			auth,
			mailer: Arc::new(mailer),
			mail_delay: Duration::from_millis(1_500),
			summarizer: Arc::new(LocalSummarizer),
			settlement: Settlement::default(),
		}
	}
}

impl std::fmt::Debug for Services {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Services")
			.field("mail_delay", &self.mail_delay)
			.field("settlement", &self.settlement)
			.finish_non_exhaustive()
	}
}

static SERVICES: RwLock<Option<Arc<Services>>> = RwLock::new(None);

/// Makes `services` the ones every operation uses from now on.
pub fn install(services: Services) {
	*SERVICES.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(services));
}

/// The installed services.
///
/// # Panics
///
/// When the server hasn't called [`install`].
pub fn get() -> Arc<Services> {
	SERVICES
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
		.expect("huddle_server installs its services before serving")
}
