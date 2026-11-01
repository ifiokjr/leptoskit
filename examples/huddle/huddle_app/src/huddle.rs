//! Everything the signed-in screens read and do, created once when someone
//! signs in.
//!
//! Every list here is a live query: `use_live` fetches it once and then
//! keeps it current from the socket, in the same `leptoskit_query` cache any
//! screen reads. Nothing names a subscription or invalidates anything; a
//! write anywhere re-runs the queries it could change, as their reader.
//!
//! [`Huddle`] is `Copy`, so handlers capture it freely.

use huddle_api::ChannelCard;
use huddle_api::InviteView;
use huddle_api::MessageView;
use huddle_api::Person;
use huddle_api::SessionState;
use huddle_api::TaskCard;
use huddle_api::TipView;
use huddle_db::Channel;
use leptoskit::auth::Account;
use leptoskit::db::Id;
use leptoskit::leptos::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::use_mutation;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::InboxHandle;
use leptoskit_rpc::client::mutation_scope;
use leptoskit_rpc::client::use_action;
use leptoskit_rpc::use_inbox;
use leptoskit_rpc::use_live;

use crate::room::Room;

/// The channel shown before the sidebar has loaded: no channel has it, so
/// every query about it is empty.
pub fn no_channel() -> Id<Channel> {
	Id::parse("00000000-0000-0000-0000-000000000000").expect("the nil ID parses")
}

/// Which part of a channel is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
	#[default]
	Chat,
	Board,
	Activity,
}

/// A message on its way: shown greyed until the live list has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
	pub channel: Id<Channel>,
	pub client_key: String,
	pub body: String,
	/// The server refused it, or couldn't be reached.
	pub failed: bool,
}

/// The signed-in screens' state.
#[derive(Clone, Copy)]
pub struct Huddle {
	pub selected: RwSignal<Id<Channel>>,
	pub tab: RwSignal<Tab>,
	pub draft: RwSignal<String>,
	pub new_channel: RwSignal<String>,
	pub new_task: RwSignal<String>,
	pub invite_email: RwSignal<String>,
	pub tip_amount: RwSignal<String>,
	pub pending: RwSignal<Vec<Pending>>,
	/// What the last action or mutation said went wrong, for the status
	/// line.
	pub problem: RwSignal<Option<String>>,
	pub show_inbox: RwSignal<bool>,
	pub room: Room,
	pub me: QueryHandle<Option<Person>, FnError>,
	pub people: QueryHandle<Vec<Person>, FnError>,
	pub channels: QueryHandle<Vec<ChannelCard>, FnError>,
	pub messages: QueryHandle<Vec<MessageView>, FnError>,
	pub members: QueryHandle<Vec<Person>, FnError>,
	pub board: QueryHandle<Vec<TaskCard>, FnError>,
	pub invites: QueryHandle<Vec<InviteView>, FnError>,
	pub tips: QueryHandle<Vec<TipView>, FnError>,
	pub session: QueryHandle<SessionState, FnError>,
	pub inbox: StoredValue<InboxHandle>,
	pub reminder: MutationHandle<huddle_api::RemindMe, String, FnError>,
	pub summarize: MutationHandle<huddle_api::SummarizeChannel, String, FnError>,
	pub sign_out_everywhere: MutationHandle<huddle_api::SignOutEverywhere, (), FnError>,
	pub read: MutationHandle<huddle_api::ReadChannel, (), FnError>,
	counter: StoredValue<u64>,
}

impl Huddle {
	/// Creates the queries, the room, and the mutations in the current
	/// reactive owner, which needs a `QueryClient`.
	#[expect(
		clippy::new_without_default,
		reason = "it subscribes to the server: not a value to make by default"
	)]
	pub fn new() -> Self {
		let selected = RwSignal::new(no_channel());
		let channel = move || selected.get();
		let me = use_live(|| huddle_api::Whoami {});
		let huddle = Self {
			selected,
			tab: RwSignal::new(Tab::Chat),
			draft: RwSignal::new(String::new()),
			new_channel: RwSignal::new(String::new()),
			new_task: RwSignal::new(String::new()),
			invite_email: RwSignal::new(String::new()),
			tip_amount: RwSignal::new("5".to_owned()),
			pending: RwSignal::new(Vec::new()),
			problem: RwSignal::new(None),
			show_inbox: RwSignal::new(false),
			room: Room::new(selected, me),
			me,
			people: use_live(|| huddle_api::People {}),
			channels: use_live(|| huddle_api::Channels {}),
			messages: use_live(move || {
				huddle_api::Messages {
					channel: channel(),
					limit: huddle_api::PAGE,
					before: None,
				}
			}),
			members: use_live(move || huddle_api::Members { channel: channel() }),
			board: use_live(move || huddle_api::Board { channel: channel() }),
			invites: use_live(move || huddle_api::Invites { channel: channel() }),
			tips: use_live(|| huddle_api::Tips {}),
			session: use_live(|| huddle_api::MySession {}),
			inbox: StoredValue::new(use_inbox()),
			reminder: use_mutation(mutation_scope::<huddle_api::RemindMe>()),
			summarize: use_action::<huddle_api::SummarizeChannel>(),
			sign_out_everywhere: use_action::<huddle_api::SignOutEverywhere>(),
			read: use_mutation(mutation_scope::<huddle_api::ReadChannel>()),
			counter: StoredValue::new(0),
		};
		huddle.follow_the_sidebar();
		huddle.mark_read_while_open();
		huddle
	}

	/// Opens the first channel once the sidebar loads, and another when the
	/// open one vanishes because someone took you out of it.
	fn follow_the_sidebar(self) {
		Effect::new(move |_| {
			let Some(channels) = self.channels.data() else {
				return;
			};
			let open = self.selected.get_untracked();

			if !channels.iter().any(|card| card.id == open) {
				self.selected
					.set(channels.first().map_or_else(no_channel, |card| card.id));
			}
		});
	}

	/// Marks the open channel read whenever its messages change.
	fn mark_read_while_open(self) {
		Effect::new(move |seen: Option<Option<String>>| {
			let latest = self
				.messages
				.data()
				.and_then(|messages| messages.last().map(|message| message.id.to_string()));
			let channel = self.selected.get_untracked();

			if latest.is_some() && seen.flatten() != latest && channel != no_channel() {
				self.read.mutate(huddle_api::ReadChannel { channel });
			}

			latest
		});
	}

	/// The open channel's card, once the sidebar has it.
	pub fn open_channel(&self) -> Option<ChannelCard> {
		let open = self.selected.get();
		self.channels
			.data()
			.and_then(|channels| channels.into_iter().find(|card| card.id == open))
	}

	/// The signed-in person's account.
	pub fn my_id(&self) -> Option<Id<Account>> {
		self.me.data().flatten().map(|me| me.id)
	}

	/// Sends the draft: it shows at once, greyed, and turns solid when the
	/// live list has the server's row with the same key.
	pub fn send(&self) {
		let body = self.draft.get_untracked().trim().to_owned();

		if body.is_empty() {
			return;
		}

		let channel = self.selected.get_untracked();
		let count = self.counter.get_value() + 1;
		self.counter.set_value(count);
		let client_key = format!("{}-{count}", leptoskit::query::now_ms());
		self.draft.set(String::new());
		self.room.stopped_typing();
		self.pending.update(|pending| {
			pending.push(Pending {
				channel,
				client_key: client_key.clone(),
				body: body.clone(),
				failed: false,
			});
		});
		self.deliver(channel, body, client_key);
	}

	/// Sends a pending message again, with the same key, so it's stored
	/// once however many times it reaches the server.
	pub fn resend(&self, client_key: &str) {
		let Some(message) = self
			.pending
			.get_untracked()
			.into_iter()
			.find(|pending| pending.client_key == client_key)
		else {
			return;
		};
		self.pending.update(|pending| {
			for item in pending
				.iter_mut()
				.filter(|item| item.client_key == client_key)
			{
				item.failed = false;
			}
		});
		self.deliver(message.channel, message.body, message.client_key);
	}

	fn deliver(&self, channel: Id<Channel>, body: String, client_key: String) {
		let pending = self.pending;
		let key = client_key.clone();
		any_spawner::Executor::spawn_local(async move {
			let sent = leptoskit_rpc::client::call(huddle_api::SendMessage {
				channel,
				body,
				client_key,
			})
			.await;

			if sent.is_err() {
				pending.update(|pending| {
					for item in pending.iter_mut().filter(|item| item.client_key == key) {
						item.failed = true;
					}
				});
			}
		});
	}

	/// The messages still on their way in the open channel.
	pub fn pending_here(&self, delivered: &[MessageView]) -> Vec<Pending> {
		let open = self.selected.get();
		self.pending
			.get()
			.into_iter()
			.filter(|pending| {
				pending.channel == open
					&& !delivered
						.iter()
						.any(|message| message.client_key == pending.client_key)
			})
			.collect()
	}

	/// Runs `op`, and shows what went wrong, if anything.
	pub fn run<O>(&self, op: O)
	where
		O: leptoskit_rpc::Operation,
	{
		let problem = self.problem;
		problem.set(None);
		any_spawner::Executor::spawn_local(async move {
			if let Err(error) = leptoskit_rpc::client::call(op).await {
				problem.set(Some(error.message().to_owned()));
			}
		});
	}
}
