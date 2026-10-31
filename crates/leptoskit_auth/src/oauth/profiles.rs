//! Reading who signed in from a provider's own API, for providers that
//! speak OAuth 2 without OpenID Connect. Each is a map from the API's JSON
//! to a [`ProviderProfile`]: JSON pointers (RFC 6901) to the subject, name,
//! username, picture, and email address, and to the flag that says whether
//! the provider checked that address. Without such a flag, an address is
//! never vouched for.

use serde_json::Value;

use super::ProviderRequest;
use super::Providers;
use crate::AuthError;
use crate::Result;
use crate::providers::ProviderProfile;

/// Where a provider's API says who signed in, and how to read it.
#[derive(Clone, Debug)]
pub(crate) struct ApiProfile {
	/// The URL to `GET` with the access token; `None` when the token
	/// endpoint's answer says who signed in, as Notion's does.
	pub url: Option<String>,
	/// Headers it needs besides the access token, such as an API version.
	pub headers: &'static [(&'static str, &'static str)],
	/// The person's stable ID: a string or a number.
	pub subject: &'static str,
	/// Their name.
	pub name: Option<&'static str>,
	/// Their username.
	pub username: Option<&'static str>,
	/// A picture of them.
	pub picture: Option<&'static str>,
	/// Their address, in the profile itself.
	pub email: Option<&'static str>,
	/// The flag that says the provider checked that address: `true`,
	/// `"true"`, or `1`. `None` when it never says, and then the address is
	/// kept for display but never trusted.
	pub verified: Option<&'static str>,
	/// A separate list of addresses, for providers that keep them apart,
	/// such as GitHub.
	pub emails: Option<EmailList>,
	/// A value the answer must hold for it to be a person, such as Notion's
	/// `/owner/user/type` of `person`: a grant owned by a bot or a whole
	/// workspace signs no one in.
	pub person: Option<(&'static str, &'static str)>,
}

/// A list of someone's addresses, of which the primary one is theirs.
#[derive(Clone, Debug)]
pub(crate) struct EmailList {
	/// The URL to `GET` with the access token.
	pub url: &'static str,
	/// Where the list is in the answer: `""` for the whole answer.
	pub list: &'static str,
	/// Each entry's address.
	pub email: &'static str,
	/// Each entry's flag for the primary address.
	pub primary: &'static str,
	/// Each entry's flag for an address the provider checked.
	pub verified: &'static str,
}

impl ApiProfile {
	/// A profile at `url`, whose subject is at `subject`, and nothing else
	/// yet.
	pub(crate) fn at(url: impl Into<String>, subject: &'static str) -> Self {
		Self {
			url: Some(url.into()),
			headers: &[],
			subject,
			name: None,
			username: None,
			picture: None,
			email: None,
			verified: None,
			emails: None,
			person: None,
		}
	}

	/// GitHub: `/user`, and the primary address from `/user/emails` with
	/// its `verified` flag, since the profile's own address may be private.
	pub(crate) fn github() -> Self {
		Self {
			headers: GITHUB_HEADERS,
			name: Some("/name"),
			username: Some("/login"),
			picture: Some("/avatar_url"),
			emails: Some(EmailList {
				url: "https://api.github.com/user/emails",
				list: "",
				email: "/email",
				primary: "/primary",
				verified: "/verified",
			}),
			..Self::at("https://api.github.com/user", "/id")
		}
	}

	/// Reads who `access` belongs to: from the profile API, or from the
	/// token endpoint's answer, `tokens`.
	pub(crate) async fn read(
		&self,
		providers: &Providers,
		access: &str,
		tokens: &Value,
		user_agent: Option<(String, String)>,
	) -> Result<ProviderProfile> {
		let request = |url: &str| {
			let mut request = self.request(url, access);
			request.headers.extend(user_agent.clone());
			request
		};
		let profile: Value = match &self.url {
			Some(url) => providers.json(request(url)).await?,
			None => tokens.clone(),
		};

		if let Some((pointer, expected)) = self.person
			&& profile.pointer(pointer).and_then(Value::as_str) != Some(expected)
		{
			return Err(AuthError::Provider(
				"the provider's grant doesn't belong to a person".into(),
			));
		}

		let subject = match profile.pointer(self.subject) {
			Some(Value::String(subject)) => subject.clone(),
			Some(Value::Number(subject)) => subject.to_string(),
			_ => String::new(),
		};

		if subject.is_empty() || subject.len() > 255 {
			return Err(AuthError::Provider(
				"the provider's profile has no usable ID".into(),
			));
		}

		let (email, email_verified) = if let Some(list) = &self.emails {
			let answer: Value = providers.json(request(list.url)).await?;
			list.primary(&answer)
		} else {
			let verified = self
				.verified
				.is_some_and(|flag| profile.pointer(flag).is_some_and(truthy));
			(text(&profile, self.email), verified)
		};

		Ok(ProviderProfile {
			subject,
			email_verified: email_verified && email.is_some(),
			email,
			name: text(&profile, self.name),
			preferred_username: text(&profile, self.username),
			picture: text(&profile, self.picture),
		})
	}

	fn request(&self, url: &str, access: &str) -> ProviderRequest {
		let mut headers = vec![
			("accept".to_owned(), "application/json".to_owned()),
			("authorization".to_owned(), format!("Bearer {access}")),
		];

		for (name, value) in self.headers {
			headers.retain(|(existing, _)| existing != name);
			headers.push(((*name).to_owned(), (*value).to_owned()));
		}

		ProviderRequest {
			method: "GET",
			url: url.to_owned(),
			headers,
			body: None,
		}
	}
}

impl EmailList {
	/// The primary address, and whether the provider checked it.
	fn primary(&self, answer: &Value) -> (Option<String>, bool) {
		let primary = answer
			.pointer(self.list)
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.find(|entry| entry.pointer(self.primary).is_some_and(truthy));
		let Some(primary) = primary else {
			return (None, false);
		};
		(
			text(primary, Some(self.email)),
			primary.pointer(self.verified).is_some_and(truthy),
		)
	}
}

const GITHUB_HEADERS: &[(&str, &str)] = &[
	("accept", "application/vnd.github+json"),
	("x-github-api-version", "2022-11-28"),
];

/// The non-empty string at `pointer`.
fn text(value: &Value, pointer: Option<&str>) -> Option<String> {
	pointer
		.and_then(|pointer| value.pointer(pointer))
		.and_then(Value::as_str)
		.map(str::trim)
		.filter(|text| !text.is_empty())
		.map(str::to_owned)
}

/// Whether a flag says yes: `true`, `"true"`, or `1`. Anything else,
/// including a missing flag, is no.
fn truthy(flag: &Value) -> bool {
	match flag {
		Value::Bool(flag) => *flag,
		Value::String(flag) => flag == "true",
		Value::Number(flag) => flag.as_u64() == Some(1),
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn only_a_flag_that_says_yes_vouches_for_an_address() {
		for yes in [json!(true), json!("true"), json!(1)] {
			assert!(truthy(&yes), "{yes}");
		}

		for no in [
			json!(false),
			json!("yes"),
			json!(0),
			json!(2),
			json!(null),
			json!([true]),
		] {
			assert!(!truthy(&no), "{no}");
		}
	}

	#[test]
	fn the_primary_address_comes_from_the_list() {
		let list = EmailList {
			url: "https://example.com/emails",
			list: "/values",
			email: "/email",
			primary: "/is_primary",
			verified: "/is_confirmed",
		};
		let answer = json!({ "values": [
			{ "email": "old@example.com", "is_primary": false, "is_confirmed": true },
			{ "email": "ada@example.com", "is_primary": true, "is_confirmed": false },
		]});
		assert_eq!(
			list.primary(&answer),
			(Some("ada@example.com".into()), false)
		);
		assert_eq!(list.primary(&json!({ "values": [] })), (None, false));
		assert_eq!(list.primary(&json!("nonsense")), (None, false));
	}

	#[test]
	fn text_is_trimmed_and_never_empty() {
		let value = json!({ "a": " Ada ", "b": "  ", "c": 1 });
		assert_eq!(text(&value, Some("/a")), Some("Ada".into()));
		assert_eq!(text(&value, Some("/b")), None);
		assert_eq!(text(&value, Some("/c")), None);
		assert_eq!(text(&value, None), None);
	}
}
