//! The inbox: notices for people, written in the caller's transaction, in a
//! live table each account reads only its own of.
//!
//! ```ignore
//! cx.notify(owner, Notice::new("Payment confirmed").kind(NoticeKind::Success)).await?;
//! ```
//!
//! A notice also goes to the app's delivery channels once it's written:
//! [`deliver_with`] adds one, such as an email for people who asked for it.
//! Remote push will plug in the same way.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use leptoskit_db::Actor;
use leptoskit_db::Connection;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;

use super::MutationCx;
use super::QueryCx;
use super::handle;
use crate::Error;
use crate::Operation;
use crate::Result;
use crate::inbox::Inbox;
use crate::inbox::InboxNotice;
use crate::inbox::Notice;
use crate::inbox::NoticeAction;
use crate::inbox::NoticeKind;

/// One notice in someone's inbox, in the `rpc` pack's `inbox` table.
///
/// Its policies let each account read and mark read only its own notices;
/// only server code writes them, through `cx.notify(...)`.
#[table(name = "inbox", pack = "rpc", live, index(account, created_at))]
#[policy(insert, |_, _| Expr::always())]
#[policy(select, update, |notice, cx| notice.account.eq(cx.setting::<Actor>()))]
pub struct InboxRow {
	pub id: Id<Self>,
	/// Whose notice it is: their account's UUID.
	#[db(max_len = 64)]
	pub account: String,
	#[db(max_len = 255)]
	pub title: String,
	#[db(max_len = 4000)]
	pub body: String,
	#[db(max_len = 2000)]
	pub link: Option<String>,
	/// The button on it.
	pub action: Option<Json<NoticeAction>>,
	/// `info`, `success`, `warning`, or `error`.
	#[db(max_len = 32)]
	pub kind: String,
	/// When its owner read it.
	pub read_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

impl InboxRow {
	/// What a client reads about it.
	#[must_use]
	pub fn to_notice(&self) -> InboxNotice {
		InboxNotice {
			id: self.id.to_string(),
			notice: Notice {
				title: self.title.clone(),
				body: self.body.clone(),
				link: self.link.clone(),
				action: self.action.as_ref().map(|action| action.0.clone()),
				kind: NoticeKind::parse(&self.kind),
			},
			read: self.read_at.is_some(),
			created_at: self.created_at.to_rfc3339(),
		}
	}
}

/// Whose inbox a notice goes to: an account's key or its [`Actor`]. `None`
/// sends nothing, so `cx.notify(cx.owner(), …)` from a job nobody signed in
/// to start does nothing.
pub trait Recipient: Send + Sync {
	/// The account's UUID, as text.
	fn account(&self) -> Option<String>;
}

impl<T: ?Sized> Recipient for Id<T> {
	fn account(&self) -> Option<String> {
		Some(self.uuid().to_string())
	}
}

impl Recipient for Actor {
	fn account(&self) -> Option<String> {
		Some(self.uuid().to_string())
	}
}

impl<R: Recipient> Recipient for Option<R> {
	fn account(&self) -> Option<String> {
		self.as_ref().and_then(Recipient::account)
	}
}

impl<R: Recipient + ?Sized> Recipient for &R {
	fn account(&self) -> Option<String> {
		(**self).account()
	}
}

/// A notice that was written, on its way to the app's delivery channels.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
	/// The notice's id in the inbox.
	pub id: String,
	/// The account's UUID.
	pub account: String,
	/// What it says.
	pub notice: Notice,
}

/// Somewhere notices go besides the inbox: email, or, later, remote push.
///
/// Delivery is best effort and happens after the notice is written: a
/// channel that fails doesn't take the notice back.
///
/// ```ignore
/// struct EmailNotices(Mailer);
///
/// impl rpc::server::inbox::Deliver for EmailNotices {
/// 	fn deliver<'a>(&'a self, delivery: &'a Delivery) -> BoxFuture<'a, rpc::Result<()>> {
/// 		Box::pin(async move { /* look up the account's address and send */ Ok(()) })
/// 	}
/// }
///
/// rpc::server::inbox::deliver_with(EmailNotices(mailer));
/// ```
pub trait Deliver: Send + Sync + 'static {
	/// Sends one notice.
	///
	/// # Errors
	///
	/// When it couldn't; the notice stays in the inbox either way.
	fn deliver<'a>(&'a self, delivery: &'a Delivery) -> BoxFuture<'a, Result<()>>;
}

static CHANNELS: RwLock<Vec<Arc<dyn Deliver>>> = RwLock::new(Vec::new());

/// Sends every notice written from now on to `channel` too.
pub fn deliver_with(channel: impl Deliver) {
	CHANNELS
		.write()
		.unwrap_or_else(PoisonError::into_inner)
		.push(Arc::new(channel));
}

/// Hands a written notice to every delivery channel.
pub(crate) async fn deliver(delivery: Delivery) {
	let channels = CHANNELS
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone();
	for channel in channels {
		let _ = channel.deliver(&delivery).await;
	}
}

/// Writes a notice into `account`'s inbox on `connection`, which may be a
/// transaction. `None` when there's no account to send it to.
pub(crate) async fn insert(
	connection: &impl Connection,
	account: &impl Recipient,
	notice: Notice,
) -> Result<Option<Delivery>> {
	let Some(account) = account.account() else {
		return Ok(None);
	};
	if notice.title.trim().is_empty() {
		return Err(Error::invalid("a notice needs a title"));
	}
	let row = NewInboxRow {
		account: account.clone(),
		title: notice.title.clone(),
		body: notice.body.clone(),
		link: notice.link.clone(),
		action: notice.action.clone().map(Json),
		kind: notice.kind.as_str().to_owned(),
		read_at: None,
	}
	.insert(connection)
	.await?;
	Ok(Some(Delivery {
		id: row.id.to_string(),
		account,
		notice,
	}))
}

impl MutationCx {
	/// Puts a notice in `account`'s inbox in this mutation's transaction:
	/// it arrives exactly when the mutation commits. It goes to the app's
	/// delivery channels after the commit.
	///
	/// ```ignore
	/// cx.notify(invite.inviter, Notice::new("Ada joined the channel")).await?;
	/// ```
	///
	/// # Errors
	///
	/// When the notice can't be written, such as one with no title.
	pub async fn notify(&self, account: impl Recipient, notice: Notice) -> Result<Option<String>> {
		let Some(delivery) = insert(self.tx(), &account, notice).await? else {
			return Ok(None);
		};
		let id = delivery.id.clone();
		self.after_commit(deliver(delivery));
		Ok(Some(id))
	}
}

/// The newest notices in the caller's inbox, and how many are unread.
pub(crate) async fn read(cx: &QueryCx, limit: u32) -> Result<Inbox> {
	let notices = InboxRow::query()
		.order_by(|notice| notice.created_at.desc())
		.limit(u64::from(limit.clamp(1, 200)))
		.all(cx.db())
		.await?;
	let unread = InboxRow::query()
		.filter(|notice| notice.read_at.is_null())
		.count(cx.db())
		.await?;
	Ok(Inbox {
		notices: notices.iter().map(InboxRow::to_notice).collect(),
		unread,
	})
}

/// Marks the caller's notice `id` read.
pub(crate) async fn mark_read(cx: &MutationCx, id: String) -> Result<()> {
	let id: Id<InboxRow> = id
		.parse()
		.map_err(|_| Error::invalid("that isn't a notice id"))?;
	let notice = InboxRow::query()
		.filter(|notice| notice.id.eq(id))
		.first(cx.tx())
		.await?
		.ok_or_else(|| Error::not_found("no such notice"))?;
	if notice.read_at.is_none() {
		InboxRow::update()
			.filter(|notice| notice.id.eq(id))
			.set(|notice| notice.read_at, leptoskit_db::now())
			.run(cx.tx())
			.await?;
	}
	Ok(())
}

/// Marks every unread notice in the caller's inbox read. How many it
/// marked.
pub(crate) async fn mark_all_read(cx: &MutationCx) -> Result<u64> {
	if !cx.signed_in() {
		return Err(Error::unauthorized("sign in to read your notices"));
	}
	InboxRow::update()
		.filter(|notice| notice.read_at.is_null())
		.set(|notice| notice.read_at, leptoskit_db::now())
		.run(cx.tx())
		.await
		.map_err(Into::into)
}

#[allow(deprecated)]
const _: () = {
	inventory::submit! {
		super::Registration {
			name: <crate::inbox::InboxArgs as Operation>::NAME,
			kind: <crate::inbox::InboxArgs as Operation>::KIND,
			module_path: module_path!(),
			handler: handle::<crate::inbox::InboxArgs>,
		}
	}
	inventory::submit! {
		super::Registration {
			name: <crate::inbox::MarkRead as Operation>::NAME,
			kind: <crate::inbox::MarkRead as Operation>::KIND,
			module_path: module_path!(),
			handler: handle::<crate::inbox::MarkRead>,
		}
	}
	inventory::submit! {
		super::Registration {
			name: <crate::inbox::MarkAllRead as Operation>::NAME,
			kind: <crate::inbox::MarkAllRead as Operation>::KIND,
			module_path: module_path!(),
			handler: handle::<crate::inbox::MarkAllRead>,
		}
	}
};
