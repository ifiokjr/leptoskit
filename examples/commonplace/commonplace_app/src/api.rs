//! Commonplace's API, the same on every host: typed calls over a
//! [`Transport`].

use std::cell::RefCell;
use std::rc::Rc;

use commonplace_db::PublicClip;
use commonplace_db::PublicNote;
use commonplace_db::PublicProject;
use commonplace_shared::ApiError;
use commonplace_shared::ClipChanges;
use commonplace_shared::ClipId;
use commonplace_shared::ClipOnPage;
use commonplace_shared::NewClipInput;
use commonplace_shared::NewProjectInput;
use commonplace_shared::NoteId;
use commonplace_shared::NoteInput;
use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectId;
use commonplace_shared::ProjectSummary;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::transport::Request;
use crate::transport::Transport;

/// Why a call failed, in words for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
	pub message: String,
	/// The server no longer knows this session.
	pub signed_out: bool,
	/// The request never reached the server, so it can be tried again.
	pub unreachable: bool,
}

impl Problem {
	fn new(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			signed_out: false,
			unreachable: false,
		}
	}
}

/// The answer of `GET /auth/session`, as much as the app needs.
#[derive(Deserialize)]
struct SessionAnswer {
	session: Option<serde_json::Value>,
}

/// The answer of `POST /auth/email/verify`.
#[derive(Deserialize)]
struct Verified {
	token: Option<String>,
}

/// The API client.
#[derive(Clone)]
pub struct Api {
	transport: Rc<dyn Transport>,
	token: Rc<RefCell<Option<String>>>,
}

impl Api {
	#[must_use]
	pub fn new(transport: impl Transport + 'static) -> Self {
		Self {
			transport: Rc::new(transport),
			token: Rc::default(),
		}
	}

	/// Whether this host keeps a bearer token, which it must store itself.
	#[must_use]
	pub fn uses_bearer(&self) -> bool {
		self.transport.uses_bearer()
	}

	/// The session token this client sends, on hosts that use one.
	#[must_use]
	pub fn token(&self) -> Option<String> {
		self.token.borrow().clone()
	}

	/// Uses a token kept from an earlier sign-in.
	pub fn set_token(&self, token: Option<String>) {
		*self.token.borrow_mut() = token;
	}

	async fn call<T: DeserializeOwned>(
		&self,
		method: &'static str,
		path: impl Into<String>,
		body: Option<&impl Serialize>,
	) -> Result<T, Problem> {
		let bytes = self.send(method, path, body).await?;
		let bytes = if bytes.is_empty() {
			b"null".to_vec()
		} else {
			bytes
		};
		serde_json::from_slice(&bytes).map_err(|_| {
			Problem::new("The server answered in a way Commonplace doesn't understand.")
		})
	}

	async fn send(
		&self,
		method: &'static str,
		path: impl Into<String>,
		body: Option<&impl Serialize>,
	) -> Result<Vec<u8>, Problem> {
		let body = body
			.map(serde_json::to_vec)
			.transpose()
			.map_err(|_| Problem::new("That couldn't be sent."))?;
		let response = self
			.transport
			.send(Request {
				method,
				path: path.into(),
				body,
				bearer: self.token(),
			})
			.await
			.map_err(|message| {
				Problem {
					unreachable: true,
					..Problem::new(message)
				}
			})?;
		if (200..300).contains(&response.status) {
			return Ok(response.body);
		}
		let message = serde_json::from_slice::<ApiError>(&response.body)
			.map(|error| error.message)
			.unwrap_or_else(|_| format!("The server answered {}.", response.status));
		Err(Problem {
			message,
			signed_out: response.status == 401,
			unreachable: false,
		})
	}

	/// Whether the current session is still signed in.
	pub async fn signed_in(&self) -> Result<bool, Problem> {
		let answer: SessionAnswer = self.call("GET", "/auth/session", None::<&()>).await?;
		Ok(answer.session.is_some())
	}

	/// Emails a sign-in code.
	pub async fn send_code(&self, email: &str) -> Result<(), Problem> {
		self.send(
			"POST",
			"/auth/email/send",
			Some(&serde_json::json!({ "email": email })),
		)
		.await?;
		Ok(())
	}

	/// Signs in with the emailed code. A bearer host keeps the token it gets
	/// back, and returns it so it can be stored.
	pub async fn verify_code(&self, email: &str, code: &str) -> Result<Option<String>, Problem> {
		let transport = if self.uses_bearer() {
			"bearer"
		} else {
			"cookie"
		};
		let verified: Verified = self
			.call(
				"POST",
				"/auth/email/verify",
				Some(&serde_json::json!({ "email": email, "code": code, "transport": transport })),
			)
			.await?;
		if self.uses_bearer() {
			self.set_token(verified.token.clone());
		}
		Ok(verified.token)
	}

	/// Ends the session on the server, and forgets the token.
	pub async fn sign_out(&self) -> Result<(), Problem> {
		let result = self.send("POST", "/auth/sign-out", None::<&()>).await;
		self.set_token(None);
		result.map(|_| ())
	}

	pub async fn projects(&self) -> Result<Vec<ProjectSummary>, Problem> {
		self.call("GET", "/api/projects", None::<&()>).await
	}

	pub async fn create_project(&self, input: &NewProjectInput) -> Result<PublicProject, Problem> {
		self.call("POST", "/api/projects", Some(input)).await
	}

	pub async fn project(&self, id: ProjectId) -> Result<ProjectDetail, Problem> {
		self.call("GET", format!("/api/projects/{id}"), None::<&()>)
			.await
	}

	pub async fn delete_project(&self, id: ProjectId) -> Result<(), Problem> {
		self.send("DELETE", format!("/api/projects/{id}"), None::<&()>)
			.await
			.map(|_| ())
	}

	pub async fn create_clip(&self, input: &NewClipInput) -> Result<PublicClip, Problem> {
		self.call("POST", "/api/clips", Some(input)).await
	}

	pub async fn update_clip(
		&self,
		id: ClipId,
		changes: &ClipChanges,
	) -> Result<PublicClip, Problem> {
		self.call("PATCH", format!("/api/clips/{id}"), Some(changes))
			.await
	}

	pub async fn delete_clip(&self, id: ClipId) -> Result<(), Problem> {
		self.send("DELETE", format!("/api/clips/{id}"), None::<&()>)
			.await
			.map(|_| ())
	}

	/// The clips taken from one page, in every project.
	pub async fn clips_on_page(&self, url: &str) -> Result<Vec<ClipOnPage>, Problem> {
		self.call(
			"GET",
			format!("/api/clips?url={}", encode(url)),
			None::<&()>,
		)
		.await
	}

	pub async fn create_note(&self, project: ProjectId, body: &str) -> Result<PublicNote, Problem> {
		self.call(
			"POST",
			format!("/api/projects/{project}/notes"),
			Some(&NoteInput { body: body.into() }),
		)
		.await
	}

	pub async fn delete_note(&self, id: NoteId) -> Result<(), Problem> {
		self.send("DELETE", format!("/api/notes/{id}"), None::<&()>)
			.await
			.map(|_| ())
	}
}

/// Percent-encodes a query value.
fn encode(value: &str) -> String {
	let mut encoded = String::with_capacity(value.len());
	for byte in value.bytes() {
		if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
			encoded.push(char::from(byte));
		} else {
			encoded.push_str(&format!("%{byte:02X}"));
		}
	}
	encoded
}

#[cfg(test)]
mod tests {
	#[test]
	fn query_values_are_encoded() {
		assert_eq!(
			super::encode("https://example.org/a b?x=1"),
			"https%3A%2F%2Fexample.org%2Fa%20b%3Fx%3D1"
		);
	}
}
