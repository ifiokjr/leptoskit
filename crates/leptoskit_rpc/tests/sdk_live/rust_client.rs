//! The live scenario for the generated Rust client; see `sdk_live.rs`. Built
//! as an example of the generated `sdk-live-client` crate.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use sdk_live_client::Client;
use sdk_live_client::SdkNote;
use sdk_live_client::SdkPresence;
use sdk_live_client::SdkRoom;
use sdk_live_client::SdkSignal;
use sdk_live_client::live::Channel;
use sdk_live_client::live::ChannelEvent;
use sdk_live_client::live::LiveOptions;
use sdk_live_client::live::LiveStatus;
use sdk_live_client::live::Subscription;
use tokio::time::Instant;

type Failure = Box<dyn std::error::Error>;

/// How long one step may wait.
const STEP: Duration = Duration::from_secs(20);

fn env(name: &str) -> String {
	std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn check(condition: bool, message: impl Into<String>) -> Result<(), Failure> {
	if condition {
		Ok(())
	} else {
		Err(message.into().into())
	}
}

/// One person's client, holding the latest value of their subscription.
struct Person {
	name: &'static str,
	api: Client,
	notes: Subscription<Vec<SdkNote>>,
	latest: Option<Vec<SdkNote>>,
	statuses: Arc<Mutex<Vec<LiveStatus>>>,
}

impl Person {
	fn new(server_url: &str, prefix: &str, name: &'static str) -> Self {
		let api = Client::with_server_url(server_url).with_live_options(
			LiveOptions::new()
				.token(move || Some(name.to_owned()))
				.backoff(Duration::from_millis(50), Duration::from_millis(200)),
		);
		let statuses = Arc::new(Mutex::new(Vec::new()));
		let seen = Arc::clone(&statuses);
		let mut status = api.live_status();
		tokio::spawn(async move {
			loop {
				let current = status.borrow_and_update().clone();
				seen.lock().unwrap().push(current);

				if status.changed().await.is_err() {
					return;
				}
			}
		});
		let notes = api.sdk_notes_watch(prefix.to_owned());
		Self {
			name,
			api,
			notes,
			latest: None,
			statuses,
		}
	}

	fn has(&self, body: &str) -> usize {
		self.latest
			.iter()
			.flatten()
			.filter(|note| note.body == body)
			.count()
	}

	/// Waits until `test` holds for the latest value.
	async fn until(&mut self, what: &str, test: impl Fn(&Self) -> bool) -> Result<(), Failure> {
		let deadline = Instant::now() + STEP;

		loop {
			if self.latest.is_some() && test(self) {
				return Ok(());
			}

			let next = tokio::time::timeout_at(deadline, self.notes.next())
				.await
				.map_err(|_| format!("{}: waited too long for {what}", self.name))?;

			match next {
				Some(Ok(notes)) => self.latest = Some(notes),
				Some(Err(error)) => eprintln!("{}: {error}", self.name),
				None => return Err(format!("{}: the subscription ended", self.name).into()),
			}
		}
	}

	/// Whether the connection was lost and came back: a `reconnecting`
	/// status, or a second connection.
	fn reconnected(&self) -> bool {
		let statuses = self.statuses.lock().unwrap();
		let mut connections: Vec<&str> = statuses
			.iter()
			.filter_map(|status| {
				match status {
					LiveStatus::Live { connection, .. } => Some(connection.as_str()),
					_ => None,
				}
			})
			.collect();
		connections.dedup();
		statuses
			.iter()
			.any(|status| matches!(status, LiveStatus::Reconnecting { .. }))
			|| connections.len() > 1
	}
}

/// Waits until the channel reports exactly `expected` as `id:name` pairs.
async fn members(
	room: &mut Channel<SdkSignal, SdkPresence>,
	expected: &str,
	who: &str,
) -> Result<(), Failure> {
	let deadline = Instant::now() + STEP;

	loop {
		let event = tokio::time::timeout_at(deadline, room.next())
			.await
			.map_err(|_| format!("{who}: waited too long for both in the room"))?;

		match event {
			Some(ChannelEvent::Presence(members)) => {
				let mut names: Vec<String> = members
					.iter()
					.map(|member| format!("{}:{}", member.id, member.state.name))
					.collect();
				names.sort();

				if names.join(",") == expected {
					return Ok(());
				}
			}
			Some(ChannelEvent::Signal { .. }) => {}
			Some(ChannelEvent::Error(error)) => eprintln!("{who} join: {error}"),
			None => return Err(format!("{who}: the channel ended").into()),
		}
	}
}

async fn scenario() -> Result<(), Failure> {
	let server_url = env("SDK_LIVE_URL");
	let prefix = env("SDK_LIVE_PREFIX");
	let mut ada = Person::new(&server_url, &prefix, "ada");
	let mut bob = Person::new(&server_url, &prefix, "bob");

	ada.until("the first value", |_| true).await?;
	bob.until("the first value", |_| true).await?;
	check(
		ada.latest.as_ref().is_some_and(Vec::is_empty),
		"a fresh database has no notes",
	)?;
	println!("ok initial");

	// Read-your-writes: when the call returns, Bob's own subscription has it.
	let one = format!("{prefix}one");
	let first = bob.api.add_sdk_note_live(one.clone()).await?;
	check(!first.is_empty(), "the mutation returns the id")?;
	let current = bob.notes.current().transpose()?.unwrap_or_default();
	check(
		current.iter().filter(|note| note.body == one).count() == 1,
		format!("Bob sees his own write: {current:?}"),
	)?;
	println!("ok read-your-writes");

	// Ada's subscription receives it as a patch.
	ada.until("Bob's note", |ada| ada.has(&one) == 1).await?;
	println!("ok patch");

	// The proxy lets this through, then cuts the connection before the answer.
	let marked = format!("{prefix}drop-me");
	let dropped = ada.api.add_sdk_note_live(marked.clone()).await?;
	check(!dropped.is_empty(), "the re-sent mutation answers")?;
	ada.until("the dropped note", |ada| ada.has(&marked) == 1)
		.await?;
	check(
		ada.reconnected(),
		format!("Ada reconnected: {:?}", ada.statuses.lock().unwrap()),
	)?;
	check(ada.has(&marked) == 1, "the re-sent mutation ran once")?;
	println!("ok reconnect");

	let two = format!("{prefix}two");
	bob.api.add_sdk_note_live(two.clone()).await?;
	ada.until("a note after the reconnect", |ada| ada.has(&two) == 1)
		.await?;
	println!("ok resumed");

	// Channels: presence from both, and Bob's signal reaches Ada.
	let room = || {
		SdkRoom {
			room: prefix.clone(),
		}
	};
	let mut ada_room = ada.api.join_sdk_room(
		room(),
		Some(SdkPresence {
			name: "Ada".to_owned(),
		}),
	);
	let mut bob_room = bob.api.join_sdk_room(
		room(),
		Some(SdkPresence {
			name: "Bob".to_owned(),
		}),
	);
	members(&mut ada_room, "ada:Ada,bob:Bob", "ada").await?;
	members(&mut bob_room, "ada:Ada,bob:Bob", "bob").await?;
	check(
		ada_room.members().len() == 2,
		"the channel keeps its members",
	)?;
	bob_room.signal(&SdkSignal {
		text: "hi".to_owned(),
	});
	let deadline = Instant::now() + STEP;

	loop {
		let event = tokio::time::timeout_at(deadline, ada_room.next())
			.await
			.map_err(|_| "waited too long for Bob's signal")?;

		match event {
			Some(ChannelEvent::Signal { message, from }) => {
				check(
					message.text == "hi" && from.as_deref() == Some("bob"),
					format!("the signal: {message:?} from {from:?}"),
				)?;
				break;
			}
			Some(ChannelEvent::Presence(_)) => {}
			Some(ChannelEvent::Error(error)) => eprintln!("ada join: {error}"),
			None => return Err("ada: the channel ended".into()),
		}
	}

	ada_room.leave();
	bob_room.leave();
	println!("ok channel");

	ada.api.live().close();
	bob.api.live().close();
	Ok(())
}

#[tokio::main]
async fn main() {
	match tokio::time::timeout(Duration::from_secs(60), scenario()).await {
		Ok(Ok(())) => {}
		Ok(Err(error)) => {
			eprintln!("{error}");
			std::process::exit(1);
		}
		Err(_) => {
			eprintln!("timed out");
			std::process::exit(1);
		}
	}
}
