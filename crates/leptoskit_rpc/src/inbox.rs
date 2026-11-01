//! The inbox as every target sees it: the notices a person receives when
//! slow work finishes, and the operations a client reads and clears them
//! with.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
use crate::Kind;
use crate::Operation;

/// What a notice says, and what tapping it does.
///
/// ```ignore
/// cx.notify(owner, Notice::new("Payment failed")
/// 	.body("The network fee was too low.")
/// 	.link(format!("/payments/{payment}"))
/// 	.action(NoticeAction::new("Retry", RetryPayment { payment }))
/// 	.kind(NoticeKind::Error)).await?;
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Notice {
	/// One line: "Payment failed".
	pub title: String,
	/// More detail, or empty.
	pub body: String,
	/// Where tapping the notice goes, as a route such as `/payments/42`.
	pub link: Option<String>,
	/// A button on the notice that runs a mutation.
	pub action: Option<NoticeAction>,
	/// How it should look.
	pub kind: NoticeKind,
}

impl Notice {
	/// A notice with a title and nothing else.
	pub fn new(title: impl Into<String>) -> Self {
		Self {
			title: title.into(),
			..Self::default()
		}
	}

	/// With a body.
	#[must_use]
	pub fn body(mut self, body: impl Into<String>) -> Self {
		self.body = body.into();
		self
	}

	/// With a link.
	#[must_use]
	pub fn link(mut self, link: impl Into<String>) -> Self {
		self.link = Some(link.into());
		self
	}

	/// With a button.
	#[must_use]
	pub fn action(mut self, action: NoticeAction) -> Self {
		self.action = Some(action);
		self
	}

	/// Of a kind.
	#[must_use]
	pub fn kind(mut self, kind: NoticeKind) -> Self {
		self.kind = kind;
		self
	}
}

/// How a notice looks.
#[derive(
	Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
	/// News: the default.
	#[default]
	Info,
	/// Something finished well.
	Success,
	/// Something needs attention.
	Warning,
	/// Something failed.
	Error,
}

impl NoticeKind {
	/// Its name: `info`, `success`, `warning`, or `error`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Info => "info",
			Self::Success => "success",
			Self::Warning => "warning",
			Self::Error => "error",
		}
	}

	/// The kind named `name`, or [`Info`](Self::Info) for a name this build
	/// doesn't know.
	#[must_use]
	pub fn parse(name: &str) -> Self {
		match name {
			"success" => Self::Success,
			"warning" => Self::Warning,
			"error" => Self::Error,
			_ => Self::Info,
		}
	}
}

/// A button on a notice: a mutation and its arguments. Tapping it runs the
/// mutation as the person tapping, under their own policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoticeAction {
	/// The button's label: "Retry".
	pub label: String,
	/// The mutation's name.
	pub operation: String,
	/// Its arguments, as JSON.
	pub arguments: Value,
}

impl NoticeAction {
	/// A button labelled `label` that runs `operation`.
	///
	/// # Panics
	///
	/// If the arguments don't serialize, which the derived `Serialize` of
	/// an operation's arguments only does for a map with keys that aren't
	/// strings.
	#[expect(
		clippy::needless_pass_by_value,
		reason = "an action is written `NoticeAction::new(\"Retry\", Op { … })`: the arguments are built for it"
	)]
	pub fn new<O: Operation>(label: impl Into<String>, operation: O) -> Self {
		Self {
			label: label.into(),
			operation: O::NAME.to_owned(),
			arguments: serde_json::to_value(&operation)
				.expect("an operation's arguments serialize, as they must to be sent"),
		}
	}

	/// The arguments as `O`, when this action runs `O`.
	#[must_use]
	pub fn decode<O: Operation>(&self) -> Option<O> {
		if self.operation != O::NAME {
			return None;
		}
		serde_json::from_value(self.arguments.clone()).ok()
	}
}

/// One notice in someone's inbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InboxNotice {
	/// Its id.
	pub id: String,
	/// What it says.
	#[serde(flatten)]
	pub notice: Notice,
	/// Whether it has been read.
	pub read: bool,
	/// When it arrived, as an RFC 3339 timestamp.
	pub created_at: String,
}

/// The signed-in person's inbox: the newest notices, and how many are
/// unread.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Inbox {
	/// The newest notices, newest first.
	pub notices: Vec<InboxNotice>,
	/// How many notices are unread, of all of them.
	pub unread: u64,
}

/// Reads the signed-in person's inbox: the query behind
/// [`use_inbox`](crate::client::use_inbox).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InboxArgs {
	/// How many notices to return, newest first; at most 200.
	pub limit: u32,
}

/// Marks one notice read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MarkRead {
	/// The notice's id.
	pub id: String,
}

/// Marks every notice in the signed-in person's inbox read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MarkAllRead {}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for InboxArgs {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::QueryCx;
	type Output = Inbox;

	const KIND: Kind = Kind::Query;
	const NAME: &'static str = "rpc.inbox";

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(self, cx: &Self::Context) -> impl Future<Output = crate::Result<Self::Output>> + Send {
		crate::server::inbox::read(cx, self.limit)
	}
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for MarkRead {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::MutationCx;
	type Output = ();

	const KIND: Kind = Kind::Mutation;
	const NAME: &'static str = "rpc.mark_read";

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(self, cx: &Self::Context) -> impl Future<Output = crate::Result<Self::Output>> + Send {
		crate::server::inbox::mark_read(cx, self.id)
	}
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl Operation for MarkAllRead {
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	type Context = crate::MutationCx;
	type Output = u64;

	const KIND: Kind = Kind::Mutation;
	const NAME: &'static str = "rpc.mark_all_read";

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	fn run(self, cx: &Self::Context) -> impl Future<Output = crate::Result<Self::Output>> + Send {
		crate::server::inbox::mark_all_read(cx)
	}
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
use std::future::Future;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_notice_builds_and_its_action_decodes() {
		let notice = Notice::new("Payment failed")
			.body("The fee was too low.")
			.link("/payments/1")
			.action(NoticeAction::new("Retry", MarkRead { id: "n1".into() }))
			.kind(NoticeKind::Error);
		assert_eq!(notice.title, "Payment failed");
		assert_eq!(notice.link.as_deref(), Some("/payments/1"));
		let action = notice.action.as_ref().unwrap();
		assert_eq!(action.operation, "rpc.mark_read");
		assert_eq!(
			action.decode::<MarkRead>(),
			Some(MarkRead { id: "n1".into() })
		);
		assert_eq!(action.decode::<MarkAllRead>(), None);
		for kind in [
			NoticeKind::Info,
			NoticeKind::Success,
			NoticeKind::Warning,
			NoticeKind::Error,
		] {
			assert_eq!(NoticeKind::parse(kind.as_str()), kind);
		}
		assert_eq!(NoticeKind::parse("shout"), NoticeKind::Info);
	}
}
