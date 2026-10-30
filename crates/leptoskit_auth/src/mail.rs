//! Sending email: the [`Mailer`] trait, and a mailbox for development and
//! tests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

/// An email to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
	/// The recipient's address.
	pub to: String,
	/// The subject line.
	pub subject: String,
	/// The plain-text body.
	pub text: String,
}

/// Why an email wasn't sent.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

/// A future a [`Mailer`] returns.
pub type MailFuture<'a> = Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;

/// Sends email, however the app does it. Implement it in a few lines over
/// a provider's HTTP API:
///
/// ```ignore
/// impl Mailer for Resend {
/// 	fn send(&self, mail: Mail) -> MailFuture<'_> {
/// 		Box::pin(async move {
/// 			self.client.post(&self.url).json(&mail_json(&mail)).send().await
/// 				.map_err(|error| MailError(error.to_string()))?;
/// 			Ok(())
/// 		})
/// 	}
/// }
/// ```
pub trait Mailer: Send + Sync + 'static {
	/// Sends `mail`, returning once the provider accepted it.
	fn send(&self, mail: Mail) -> MailFuture<'_>;
}

/// Keeps every email instead of sending it: the development mailbox, and
/// the way tests read the codes they were sent.
#[derive(Clone, Debug, Default)]
pub struct Mailbox {
	messages: Arc<Mutex<Vec<Mail>>>,
}

impl Mailbox {
	/// An empty mailbox.
	pub fn new() -> Self {
		Self::default()
	}

	/// Every email sent so far, oldest first.
	pub fn messages(&self) -> Vec<Mail> {
		self.lock().clone()
	}

	/// The newest email sent to `address`.
	pub fn latest_to(&self, address: &str) -> Option<Mail> {
		self.lock()
			.iter()
			.rev()
			.find(|mail| mail.to.eq_ignore_ascii_case(address))
			.cloned()
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Mail>> {
		// A panic while holding the lock can't leave a `Vec` half-pushed.
		self.messages
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}
}

impl Mailer for Mailbox {
	fn send(&self, mail: Mail) -> MailFuture<'_> {
		self.lock().push(mail);
		Box::pin(async { Ok(()) })
	}
}
