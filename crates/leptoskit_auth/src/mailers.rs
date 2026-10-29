//! Mailers that send for real: over SMTP, or as a signed webhook to a URL
//! you control, which then sends through any provider.

#[cfg(feature = "smtp")]
pub use smtp::SmtpMailer;
#[cfg(feature = "webhook")]
pub use webhook::WebhookMailer;

#[cfg(feature = "webhook")]
mod webhook {
	use base64::Engine;
	use base64::engine::general_purpose::STANDARD;
	use hmac::Hmac;
	use hmac::Mac;
	use sha2::Sha256;

	use crate::AuthError;
	use crate::Mail;
	use crate::MailError;
	use crate::MailFuture;
	use crate::Mailer;
	use crate::secrets::random;

	/// Posts each email as JSON to a URL you control, signed the way
	/// [Standard Webhooks](https://www.standardwebhooks.com) describes, so
	/// the receiver can check it came from your server:
	///
	/// ```text
	/// POST <url>
	/// webhook-id: msg_…
	/// webhook-timestamp: 1767225600
	/// webhook-signature: v1,<base64 HMAC-SHA-256 of "{id}.{timestamp}.{body}">
	///
	/// {"type":"email","from":"…","to":"…","subject":"…","text":"…"}
	/// ```
	///
	/// Any answer but a 2xx status is a failure to send.
	#[derive(Clone)]
	pub struct WebhookMailer {
		client: reqwest::Client,
		url: String,
		secret: Vec<u8>,
		from: Option<String>,
	}

	impl std::fmt::Debug for WebhookMailer {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("WebhookMailer").finish_non_exhaustive()
		}
	}

	impl WebhookMailer {
		/// Posts to `url`, signing with `secret`: either the raw secret, or
		/// `whsec_` and its base64, as webhook services print it.
		///
		/// # Errors
		///
		/// [`AuthError::Config`] for a `whsec_` secret that isn't base64.
		pub fn new(
			url: impl Into<String>,
			secret: &str,
			from: Option<String>,
		) -> Result<Self, AuthError> {
			let secret = match secret.strip_prefix("whsec_") {
				Some(encoded) => {
					STANDARD.decode(encoded).map_err(|_| {
						AuthError::Config("the webhook secret after `whsec_` isn't base64".into())
					})?
				}
				None => secret.as_bytes().to_vec(),
			};

			Ok(Self {
				client: reqwest::Client::new(),
				url: url.into(),
				secret,
				from,
			})
		}

		/// The headers that sign `body`, sent as message `id` at `timestamp`.
		pub(crate) fn signature(&self, id: &str, timestamp: u64, body: &str) -> String {
			let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.secret)
				.expect("HMAC takes any key length");
			mac.update(format!("{id}.{timestamp}.{body}").as_bytes());
			format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
		}

		async fn post(&self, mail: Mail) -> Result<(), MailError> {
			let id = format!(
				"msg_{}",
				base64::engine::general_purpose::URL_SAFE_NO_PAD
					.encode(random::<16>().map_err(|error| MailError(error.to_string()))?)
			);
			let timestamp = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_or(0, |elapsed| elapsed.as_secs());
			let body = serde_json::json!({
				"type": "email",
				"from": self.from,
				"to": mail.to,
				"subject": mail.subject,
				"text": mail.text,
			})
			.to_string();
			let response = self
				.client
				.post(&self.url)
				.header("content-type", "application/json")
				.header("webhook-id", &id)
				.header("webhook-timestamp", timestamp.to_string())
				.header("webhook-signature", self.signature(&id, timestamp, &body))
				.body(body)
				.send()
				.await
				// The URL may hold credentials, so errors never include it.
				.map_err(|error| {
					MailError(format!("the webhook failed: {}", error.without_url()))
				})?;
			let status = response.status();

			if status.is_success() {
				Ok(())
			} else {
				Err(MailError(format!("the webhook answered {status}")))
			}
		}
	}

	impl Mailer for WebhookMailer {
		fn send(&self, mail: Mail) -> MailFuture<'_> {
			Box::pin(self.post(mail))
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn signatures_follow_standard_webhooks() {
			// The example from the Standard Webhooks specification.
			let mailer = WebhookMailer::new(
				"https://example.com",
				"whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw",
				None,
			)
			.unwrap();
			let signature = mailer.signature(
				"msg_p5jXN8AQM9LWM0D4loKWxJek",
				1_614_265_330,
				r#"{"test": 2432232314}"#,
			);
			assert_eq!(signature, "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");

			let raw = WebhookMailer::new("https://example.com", "plain", None).unwrap();
			assert!(raw.signature("id", 1, "{}").starts_with("v1,"));
			assert!(WebhookMailer::new("https://example.com", "whsec_!!", None).is_err());
			assert_eq!(format!("{raw:?}"), "WebhookMailer { .. }");
		}
	}
}

#[cfg(feature = "smtp")]
mod smtp {
	use lettre::AsyncSmtpTransport;
	use lettre::AsyncTransport;
	use lettre::Message;
	use lettre::Tokio1Executor;
	use lettre::message::Mailbox as Address;

	use crate::AuthError;
	use crate::Mail;
	use crate::MailError;
	use crate::MailFuture;
	use crate::Mailer;

	/// Sends over SMTP, with TLS.
	#[derive(Clone)]
	pub struct SmtpMailer {
		transport: AsyncSmtpTransport<Tokio1Executor>,
		from: Address,
	}

	impl std::fmt::Debug for SmtpMailer {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.debug_struct("SmtpMailer")
				.field("from", &self.from.to_string())
				.finish_non_exhaustive()
		}
	}

	impl SmtpMailer {
		/// Sends through the server at `url`, such as
		/// `smtps://user:password@smtp.example.com` (TLS from the start) or
		/// `smtp://user:password@smtp.example.com:587?tls=required`
		/// (STARTTLS), from `from`, such as `My App <hello@myapp.com>`.
		///
		/// # Errors
		///
		/// [`AuthError::Config`] for a URL or address that doesn't parse. The
		/// message never repeats the URL, which holds a password.
		pub fn new(url: &str, from: &str) -> Result<Self, AuthError> {
			let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
				.map_err(|_| AuthError::Config("the SMTP URL doesn't parse".into()))?
				.build();
			let from = from.parse().map_err(|_| {
				AuthError::Config(format!("`{from}` isn't an address to send from"))
			})?;

			Ok(Self { transport, from })
		}

		async fn deliver(&self, mail: Mail) -> Result<(), MailError> {
			let to: Address = mail
				.to
				.parse()
				.map_err(|_| MailError(format!("`{}` isn't an address", mail.to)))?;
			let message = Message::builder()
				.from(self.from.clone())
				.to(to)
				.subject(mail.subject)
				.body(mail.text)
				.map_err(|error| MailError(error.to_string()))?;
			self.transport
				.send(message)
				.await
				.map_err(|error| MailError(format!("the SMTP server refused: {error}")))?;
			Ok(())
		}
	}

	impl Mailer for SmtpMailer {
		fn send(&self, mail: Mail) -> MailFuture<'_> {
			Box::pin(self.deliver(mail))
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[tokio::test]
		async fn smtp_settings_are_checked_without_echoing_them() {
			let error = SmtpMailer::new("not a url", "a@b.c").unwrap_err();
			assert_eq!(
				error.to_string(),
				"auth is misconfigured: the SMTP URL doesn't parse"
			);

			let error =
				SmtpMailer::new("smtps://user:hunter2@smtp.example.com", "nope").unwrap_err();
			assert!(!error.to_string().contains("hunter2"));

			let mailer = SmtpMailer::new("smtp://127.0.0.1:1", "Notes <hi@notes.example>").unwrap();
			assert!(format!("{mailer:?}").contains("hi@notes.example"));
			let bad = mailer
				.deliver(Mail {
					to: "not an address".into(),
					subject: String::new(),
					text: String::new(),
				})
				.await
				.unwrap_err();
			assert!(bad.to_string().contains("isn't an address"));
			let refused = mailer
				.send(Mail {
					to: "ada@example.com".into(),
					subject: "Hi".into(),
					text: "Hello".into(),
				})
				.await
				.unwrap_err();
			assert!(refused.to_string().starts_with("the SMTP server refused"));
		}
	}
}
