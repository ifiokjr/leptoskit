//! Huddle's tables. Every one is `live`, so a write from anywhere reaches
//! the screens watching it, and every one has a policy, so nobody's screen
//! ever receives a row they couldn't fetch.
//!
//! The rules hang off one idea: **you see a channel, and everything in it,
//! when you're a member.** Adding someone to a private channel makes it
//! appear in their sidebar, and removing them makes it vanish, because the
//! policies below ask `cx.exists(…)` about `memberships`, and a write there
//! re-runs every query those policies gated.
//!
//! Clients compile only the `Public…` types and the IDs. After changing a
//! table, run `leptoskit db migration create <name>` in `examples/huddle`
//! and commit what it writes under `migrations/`.

use leptoskit_auth::Account;
#[cfg(feature = "server")]
use leptoskit_auth::AuthContext;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

// Sign-in keeps its tables (accounts, sessions, codes, its audit trail) in
// this database, under `auth`.
leptoskit_db::packs!(leptoskit_auth);
// Background work and the inbox live in the `rpc` pack: `rpc.jobs`,
// `rpc.inbox`, and the receipts that make a retried mutation run once.
#[cfg(feature = "server")]
leptoskit_db::packs!(leptoskit_rpc);

/// The board's columns, left to right.
pub const COLUMNS: [&str; 3] = ["todo", "doing", "done"];

/// Someone with an account. The row shares its key with their
/// `auth::Account`, so deleting the account deletes the person.
///
/// Everyone signed in sees everyone's name, to add them to channels. Only
/// an admin changes a row: the first person to sign in is one.
#[table(name = "users", public, live)]
#[policy(select, |user, cx| cx.signed_in(|_| user.id.is_not_null()))]
#[policy(update, |user, cx| cx.signed_in(|me| user.id.is_not_null() & cx.exists(User::query()
	.filter(|admin| admin.id.eq(me.account) & admin.is_admin.eq(true)))),
	message = "Only an admin can change someone's account")]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	/// What Huddle calls them, from their address until they change it.
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	/// The address they signed in with, which invites are matched against.
	#[db(private, max_len = 320)]
	pub email: String,
	/// Whether they may sign people out everywhere.
	#[db(default = false)]
	pub is_admin: bool,
	/// When an admin last signed them out everywhere. Their open apps watch
	/// this row, so they drop to the sign-in screen when it changes.
	pub signed_out_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A conversation, with its own board.
///
/// A channel is visible to its members, and to whoever made it, who decides
/// who else is in it.
#[table(name = "channels", public, live)]
#[policy(select, |channel, cx| cx.signed_in(|me| channel.created_by.eq(me.account)
	| cx.exists(Membership::query()
		.filter(|member| member.channel_id.eq(channel.id) & member.user_id.eq(me.account)))))]
#[policy(insert, |channel, cx| cx.signed_in(|me| channel.created_by.eq(me.account)))]
#[policy(update, |channel, cx| cx.signed_in(|me| cx.exists(Membership::query()
	.filter(|member| member.channel_id.eq(channel.id) & member.user_id.eq(me.account)))),
	message = "Only members can change a channel")]
pub struct Channel {
	pub id: Id<Channel>,
	/// Such as `general` or `launch`, shown as `#general`.
	#[db(min_len = 1, max_len = 40, regex = "^[a-z0-9][a-z0-9-]*$")]
	pub name: String,
	/// Whether only the people added to it may join. Public channels are
	/// still membership-based; everyone joins `#general` when they sign up.
	#[db(default = false)]
	pub private: bool,
	pub created_by: Id<Account>,
	/// The last summary someone pinned with "Summarize this channel".
	#[db(max_len = 2000)]
	pub summary: Option<String>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Someone in a channel, and how far they've read.
///
/// Members see each other. The channel's creator adds and removes people;
/// anyone may leave, and keeps their own read position.
#[table(
	name = "memberships",
	public,
	live,
	unique(channel_id, user_id),
	index(user_id)
)]
#[policy(select, |member, cx| cx.signed_in(|me| member.user_id.eq(me.account)
	| cx.exists(Membership::query()
		.filter(|other| other.channel_id.eq(member.channel_id) & other.user_id.eq(me.account)))))]
#[policy(insert, |member, cx| cx.signed_in(|me| cx.exists(Channel::query()
	.filter(|channel| channel.id.eq(member.channel_id) & channel.created_by.eq(me.account)))),
	message = "Only the channel's creator can add people")]
#[policy(update, |member, cx| cx.signed_in(|me| member.user_id.eq(me.account)))]
#[policy(delete, |member, cx| cx.signed_in(|me| member.user_id.eq(me.account)
	| cx.exists(Channel::query()
		.filter(|channel| channel.id.eq(member.channel_id) & channel.created_by.eq(me.account)))),
	message = "Only the channel's creator can remove people")]
pub struct Membership {
	pub id: Id<Membership>,
	#[db(on_delete = cascade)]
	pub channel_id: Id<Channel>,
	#[db(on_delete = cascade)]
	pub user_id: Id<Account>,
	/// Messages after this are unread.
	pub last_read_at: Timestamp,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Something someone said in a channel.
#[table(
	name = "messages",
	public,
	live,
	index(channel_id, created_at),
	unique(author_id, client_key)
)]
#[policy(select, |message, cx| cx.signed_in(|me| cx.exists(Membership::query()
	.filter(|member| member.channel_id.eq(message.channel_id) & member.user_id.eq(me.account)))))]
#[policy(insert, |message, cx| cx.signed_in(|me| message.author_id.eq(me.account)
	& cx.exists(Membership::query()
		.filter(|member| member.channel_id.eq(message.channel_id) & member.user_id.eq(me.account)))),
	message = "Only members can post in a channel")]
pub struct Message {
	pub id: Id<Message>,
	#[db(on_delete = cascade)]
	pub channel_id: Id<Channel>,
	#[db(on_delete = cascade)]
	pub author_id: Id<Account>,
	#[db(min_len = 1, max_len = 4000)]
	pub body: String,
	/// The sender's own name for the message, made before it was sent, so
	/// their screen can swap its greyed-out copy for this row.
	#[db(min_len = 1, max_len = 64)]
	pub client_key: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A card on a channel's board.
#[table(name = "tasks", public, live, index(channel_id))]
#[policy(all, |task, cx| cx.signed_in(|me| cx.exists(Membership::query()
	.filter(|member| member.channel_id.eq(task.channel_id) & member.user_id.eq(me.account)))),
	message = "Only members can change a channel's board")]
pub struct Task {
	pub id: Id<Task>,
	#[db(on_delete = cascade)]
	pub channel_id: Id<Channel>,
	#[db(min_len = 1, max_len = 200)]
	pub title: String,
	/// `todo`, `doing`, or `done`.
	#[db(one_of("todo", "doing", "done"), default = "todo")]
	pub column: String,
	/// Its place in the column, from 0. Moves renumber the whole column.
	pub position: i64,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An invitation by email, which a background job sends.
#[table(name = "invites", public, live, index(channel_id), index(email))]
#[policy(select, |invite, cx| cx.signed_in(|me| invite.invited_by.eq(me.account)
	| cx.exists(Membership::query()
		.filter(|member| member.channel_id.eq(invite.channel_id) & member.user_id.eq(me.account)))))]
#[policy(insert, update, |invite, cx| cx.signed_in(|me| invite.invited_by.eq(me.account)
	& cx.exists(Membership::query()
		.filter(|member| member.channel_id.eq(invite.channel_id) & member.user_id.eq(me.account)))),
	message = "Only members can invite people to a channel")]
pub struct Invite {
	pub id: Id<Invite>,
	#[db(on_delete = cascade)]
	pub channel_id: Id<Channel>,
	#[db(on_delete = cascade)]
	pub invited_by: Id<Account>,
	#[db(min_len = 3, max_len = 320)]
	pub email: String,
	/// `sending`, `sent`, or `failed`.
	#[db(one_of("sending", "sent", "failed"), default = "sending")]
	pub status: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A tip from one teammate to another, settled by a background job against
/// Huddle's simulated settlement service.
#[table(name = "tips", public, live, index(from_id), index(to_id))]
#[policy(select, |tip, cx| cx.signed_in(|me| tip.from_id.eq(me.account) | tip.to_id.eq(me.account)))]
#[policy(insert, update, |tip, cx| cx.signed_in(|me| tip.from_id.eq(me.account)),
	message = "Only the sender can change a tip")]
pub struct Tip {
	pub id: Id<Tip>,
	#[db(on_delete = cascade)]
	pub from_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub to_id: Id<Account>,
	/// How many coins: Huddle's pretend currency.
	#[db(min = 1, max = 1000)]
	pub amount: i64,
	/// `pending`, `submitted`, `confirmed`, or `failed`.
	#[db(
		one_of("pending", "submitted", "confirmed", "failed"),
		default = "pending"
	)]
	pub status: String,
	/// The simulated service's receipt, once it accepted the tip.
	#[db(max_len = 200)]
	pub receipt: Option<String>,
	/// Why it failed, when it did.
	#[db(max_len = 500)]
	pub reason: Option<String>,
	/// How many times it was sent, counting retries from the app.
	#[db(default = 1)]
	pub attempt: i64,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Sets up a new person in the transaction that creates their account: if
/// this fails, no account is created either.
///
/// - Their name comes from the provider, or from their address.
/// - The first person to sign in is the admin, and starts `#general`.
/// - Everyone joins `#general`, and every channel an invite to their address
///   was sent for.
#[cfg(feature = "server")]
impl leptoskit_auth::OnSignUp for User {
	async fn on_sign_up(
		tx: &leptoskit_db::Tx,
		account: Id<Account>,
		profile: &leptoskit_auth::Profile,
	) -> leptoskit_auth::Result<()> {
		let email = profile
			.email
			.as_ref()
			.map(ToString::to_string)
			.unwrap_or_default();
		let name = profile.name.clone().unwrap_or_else(|| name_from(&email));
		let first = !User::query().exists(tx).await?;
		NewUser {
			id: account,
			name,
			email: email.clone(),
			is_admin: first,
			signed_out_at: None,
		}
		.insert(tx)
		.await?;

		let general = match Channel::query()
			.filter(|channel| channel.name.eq("general"))
			.first(tx)
			.await?
		{
			Some(channel) => channel.id,
			None => {
				NewChannel {
					name: "general".to_owned(),
					private: false,
					created_by: account,
					summary: None,
				}
				.insert(tx)
				.await?
				.id
			}
		};
		let mut channels = vec![general];
		let invited = Invite::query()
			.filter(|invite| invite.email.eq(email.to_lowercase()))
			.all(tx)
			.await?;
		channels.extend(invited.into_iter().map(|invite| invite.channel_id));
		channels.sort_unstable();
		channels.dedup();

		for channel_id in channels {
			NewMembership {
				channel_id,
				user_id: account,
				last_read_at: leptoskit_db::now(),
			}
			.insert(tx)
			.await?;
		}

		Ok(())
	}
}

/// `ada.lovelace@example.com` becomes `Ada Lovelace`.
#[cfg(feature = "server")]
fn name_from(email: &str) -> String {
	let local = email.split('@').next().unwrap_or_default();
	let words: Vec<String> = local
		.split(['.', '_', '-', '+'])
		.filter(|word| !word.is_empty())
		.map(|word| {
			let mut characters = word.chars();
			characters.next().map_or_else(String::new, |first| {
				first.to_uppercase().chain(characters).collect()
			})
		})
		.collect();

	if words.is_empty() {
		"New teammate".to_owned()
	} else {
		words.join(" ")
	}
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the server to apply at startup.
#[cfg(feature = "server")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("../migrations");

/// The same migrations written for Postgres, for running several servers
/// on one database: `leptoskit db migration create` with `[db] dialect =
/// "postgres"` and `migrations = "migrations-postgres"` writes them.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("../migrations-postgres");
