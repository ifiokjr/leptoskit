//! The open channel's room: who's here, and who's typing.
//!
//! `use_channel` joins with the default presence. Huddle wants each avatar
//! to carry a name, so it joins through the live client itself, with a
//! [`Here`] for this person, and follows the same events `use_channel`
//! does: presence on every join and leave, and signals at most once.

use huddle_api::Here;
use huddle_api::Person;
use huddle_api::Signal;
use huddle_db::Channel;
use leptoskit::db::Id;
use leptoskit::leptos::prelude::*;
use leptoskit::query::QueryHandle;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::ChannelEvent;

use crate::huddle::no_channel;

/// How long "is typing" shows after the last keystroke signal.
const TYPING_MS: u64 = 4_000;

/// How often a typing person tells the others again.
const RESEND_MS: u64 = 2_000;

/// Someone typing, and until when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typing {
	pub user: String,
	pub name: String,
	pub until_ms: u64,
}

/// The room's state.
#[derive(Clone, Copy)]
pub struct Room {
	key: StoredValue<Option<String>>,
	here: RwSignal<Vec<Here>>,
	typing: RwSignal<Vec<Typing>>,
	/// Bumped when a typing indicator should have expired, so the screen
	/// renders again without it.
	tick: RwSignal<u64>,
	last_sent: StoredValue<u64>,
	me: QueryHandle<Option<Person>, FnError>,
}

impl Room {
	/// Joins the room of whichever channel `selected` names, as `me`, and
	/// moves when it changes.
	pub fn new(selected: RwSignal<Id<Channel>>, me: QueryHandle<Option<Person>, FnError>) -> Self {
		let room = Self {
			key: StoredValue::new(None),
			here: RwSignal::new(Vec::new()),
			typing: RwSignal::new(Vec::new()),
			tick: RwSignal::new(0),
			last_sent: StoredValue::new(0),
			me,
		};

		Effect::new(move |_| {
			let channel = selected.get();
			let me = me.data().flatten();

			if !leptoskit_rpc::client::live::available() {
				return;
			}

			let live = leptoskit_rpc::client::live::client();

			if let Some(previous) = room.key.get_value() {
				live.leave(&previous);
				room.key.set_value(None);
				room.here.set(Vec::new());
				room.typing.set(Vec::new());
			}

			let Some(me) = me.filter(|_| channel != no_channel()) else {
				return;
			};
			let here = Here {
				user: me.id.to_string(),
				name: me.name,
			};
			let (key, events) = live.join(huddle_api::Room { channel }, Some(here));
			room.key.set_value(Some(key));
			any_spawner::Executor::spawn_local(room.follow(events));
		});

		room
	}

	async fn follow(self, mut events: futures::channel::mpsc::UnboundedReceiver<ChannelEvent>) {
		use futures::StreamExt;

		while let Some(event) = events.next().await {
			match event {
				ChannelEvent::Presence(members) => {
					self.here.set(
						members
							.into_iter()
							.filter_map(|member| serde_json::from_value(member.state).ok())
							.collect(),
					);
				}
				ChannelEvent::Signal { message, .. } => {
					if let Ok(signal) = serde_json::from_value::<Signal>(message) {
						self.heard(signal);
					}
				}
				ChannelEvent::Error(_) => {}
			}
		}
	}

	fn heard(self, signal: Signal) {
		match signal {
			Signal::Typing { user, name } => {
				let until_ms = leptoskit::query::now_ms() + TYPING_MS;
				self.typing.update(|typing| {
					typing.retain(|someone| someone.user != user);
					typing.push(Typing {
						user,
						name,
						until_ms,
					});
				});
				// In the browser this wakes the screen when the indicator
				// expires; native renders on its next turn either way.
				let tick = self.tick;
				any_spawner::Executor::spawn_local(async move {
					leptoskit::query::sleep(std::time::Duration::from_millis(TYPING_MS)).await;
					tick.update(|tick| *tick += 1);
				});
			}
			Signal::Stopped { user } => {
				self.typing
					.update(|typing| typing.retain(|someone| someone.user != user));
			}
		}
	}

	/// Who's here now, without duplicates when one person has two devices
	/// open.
	pub fn here(&self) -> Vec<Here> {
		let mut here = self.here.get();
		here.sort_by(|a, b| a.name.cmp(&b.name).then(a.user.cmp(&b.user)));
		here.dedup_by(|a, b| a.user == b.user);
		here
	}

	/// "Ada is typing…", or `None`, leaving out this person.
	pub fn typing_line(&self) -> Option<String> {
		self.tick.track();
		let now = leptoskit::query::now_ms();
		let me = self.me.data().flatten().map(|me| me.id.to_string());
		let names: Vec<String> = self
			.typing
			.get()
			.into_iter()
			.filter(|someone| someone.until_ms > now && Some(&someone.user) != me.as_ref())
			.map(|someone| someone.name)
			.collect();

		match names.as_slice() {
			[] => None,
			[one] => Some(format!("{one} is typing…")),
			[one, two] => Some(format!("{one} and {two} are typing…")),
			_ => Some("Several people are typing…".to_owned()),
		}
	}

	/// Tells the others this person is typing, at most every two seconds.
	pub fn typing(&self) {
		let now = leptoskit::query::now_ms();

		if now.saturating_sub(self.last_sent.get_value()) < RESEND_MS {
			return;
		}

		self.last_sent.set_value(now);
		self.signal(|me| {
			Signal::Typing {
				user: me.id.to_string(),
				name: me.name.clone(),
			}
		});
	}

	/// Tells the others this person stopped.
	pub fn stopped_typing(&self) {
		self.last_sent.set_value(0);
		self.signal(|me| {
			Signal::Stopped {
				user: me.id.to_string(),
			}
		});
	}

	fn signal(&self, message: impl FnOnce(&Person) -> Signal) {
		let (Some(key), Some(me)) = (self.key.get_value(), self.me.get_untracked_data()) else {
			return;
		};

		if leptoskit_rpc::client::live::available() {
			leptoskit_rpc::client::live::client().signal::<huddle_api::Room>(&key, &message(&me));
		}
	}
}

/// Reads a query's value without subscribing the caller to it.
trait UntrackedData {
	fn get_untracked_data(&self) -> Option<Person>;
}

impl UntrackedData for QueryHandle<Option<Person>, FnError> {
	fn get_untracked_data(&self) -> Option<Person> {
		untrack(|| self.data().flatten())
	}
}
