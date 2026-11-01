//! Channels across servers: with a relay, each server publishes its own
//! members' signals and presence to the others over NATS, and shows the
//! others' members alongside its own.
//!
//! Signals are at most once, as within one server. Presence is each
//! server's own members of each channel, sent whenever they change and
//! again every [`ROOMS`] in full, so a server that missed a message, or
//! started late, catches up; a server not heard from for three of those is
//! taken to have stopped, and its members leave.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::Weak;
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tokio::time::Instant;

use super::Live;
use super::Outbound;
use super::Shared;
use crate::live::PresenceMember;
use crate::live::ServerFrame;
use crate::server::Relay;

/// How often each server sends every channel's members in full.
const ROOMS: Duration = Duration::from_secs(10);

/// What servers tell each other.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Wire {
	/// A member of `channel` on `instance` sent a signal.
	Signal {
		instance: String,
		channel: String,
		from: Option<String>,
		message: Value,
	},
	/// `instance`'s members of `channel` are now these.
	Presence {
		instance: String,
		channel: String,
		members: Vec<PresenceMember>,
	},
	/// Every channel `instance` has members in, and who.
	Rooms {
		instance: String,
		rooms: BTreeMap<String, Vec<PresenceMember>>,
	},
}

/// This server's view of the others' channels.
pub(super) struct Cluster {
	instance: String,
	client: async_nats::Client,
	subject: String,
	state: Mutex<Remote>,
}

#[derive(Default)]
struct Remote {
	/// Each channel's members on each other server.
	rooms: HashMap<String, BTreeMap<String, Vec<PresenceMember>>>,
	/// When each other server was last heard from.
	heard: HashMap<String, Instant>,
}

impl Cluster {
	/// Joins the others on `relay`'s channel subject for `live`'s database.
	pub(super) async fn start(live: &Live, relay: &Relay) -> Option<Arc<Self>> {
		let names = relay.names(&live.inner.db).await.ok()?;
		let subject = names.channels.clone();
		let client = relay.client().clone();
		let messages = client.subscribe(subject.clone()).await.ok()?;
		let cluster = Arc::new(Self {
			instance: crate::server::relay::instance_id(),
			client,
			subject,
			state: Mutex::new(Remote::default()),
		});
		tokio::spawn(listen(
			Arc::downgrade(&cluster),
			Arc::downgrade(&live.inner.shared),
			messages,
		));
		tokio::spawn(heartbeat(
			Arc::downgrade(&cluster),
			Arc::downgrade(&live.inner.shared),
		));
		Some(cluster)
	}

	fn state(&self) -> MutexGuard<'_, Remote> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// The other servers' members of `channel`.
	pub(super) fn members(&self, channel: &str) -> Vec<PresenceMember> {
		self.state()
			.rooms
			.get(channel)
			.into_iter()
			.flat_map(|servers| servers.values().flatten().cloned())
			.collect()
	}

	fn send(&self, wire: &Wire) {
		let Ok(payload) = serde_json::to_vec(wire) else {
			return;
		};
		let client = self.client.clone();
		let subject = self.subject.clone();
		// Publishing only queues the message, but it's async; nothing waits
		// for it, as nothing waits for a signal on one server either.
		tokio::spawn(async move {
			let _ = client.publish(subject, payload.into()).await;
		});
	}

	/// Tells the others about a signal from one of this server's members.
	pub(super) fn signal(&self, channel: &str, from: Option<String>, message: &Value) {
		self.send(&Wire::Signal {
			instance: self.instance.clone(),
			channel: channel.to_owned(),
			from,
			message: message.clone(),
		});
	}

	/// Tells the others who this server's members of `channel` are.
	pub(super) fn presence(&self, channel: &str, members: Vec<PresenceMember>) {
		self.send(&Wire::Presence {
			instance: self.instance.clone(),
			channel: channel.to_owned(),
			members,
		});
	}

	/// Takes in what another server said, and returns the channels whose
	/// members changed.
	fn receive(&self, wire: Wire) -> Vec<String> {
		let mut state = self.state();

		match wire {
			Wire::Signal { .. } => Vec::new(),
			Wire::Presence {
				instance,
				channel,
				members,
			} => {
				state.heard.insert(instance.clone(), Instant::now());
				let changed = state.set(&channel, &instance, members);
				if changed { vec![channel] } else { Vec::new() }
			}
			Wire::Rooms { instance, rooms } => {
				state.heard.insert(instance.clone(), Instant::now());
				let mut changed = Vec::new();
				let before: Vec<String> = state
					.rooms
					.iter()
					.filter(|(_, servers)| servers.contains_key(&instance))
					.map(|(channel, _)| channel.clone())
					.collect();

				for channel in before {
					if !rooms.contains_key(&channel) && state.set(&channel, &instance, Vec::new()) {
						changed.push(channel);
					}
				}

				for (channel, members) in rooms {
					if state.set(&channel, &instance, members) {
						changed.push(channel);
					}
				}

				changed
			}
		}
	}

	/// Forgets servers not heard from in a while, and returns the channels
	/// their members left.
	fn expire(&self) -> Vec<String> {
		let mut state = self.state();
		let now = Instant::now();
		let gone: Vec<String> = state
			.heard
			.iter()
			.filter(|(_, heard)| now.duration_since(**heard) > ROOMS * 3)
			.map(|(instance, _)| instance.clone())
			.collect();
		let mut changed = Vec::new();

		for instance in gone {
			state.heard.remove(&instance);
			let channels: Vec<String> = state.rooms.keys().cloned().collect();

			for channel in channels {
				if state.set(&channel, &instance, Vec::new()) {
					changed.push(channel);
				}
			}
		}

		changed
	}
}

impl Remote {
	/// Sets `instance`'s members of `channel`; whether that changed them.
	fn set(&mut self, channel: &str, instance: &str, members: Vec<PresenceMember>) -> bool {
		let servers = self.rooms.entry(channel.to_owned()).or_default();
		let changed = if members.is_empty() {
			servers.remove(instance).is_some()
		} else {
			servers.insert(instance.to_owned(), members.clone()) != Some(members)
		};

		if servers.is_empty() {
			self.rooms.remove(channel);
		}

		changed
	}
}

/// Hands what the others say to this server's members, until the server
/// or the subscription goes.
async fn listen(
	cluster: Weak<Cluster>,
	shared: Weak<Shared>,
	mut messages: async_nats::Subscriber,
) {
	while let Some(message) = messages.next().await {
		let (Some(cluster), Some(shared)) = (cluster.upgrade(), shared.upgrade()) else {
			return;
		};
		let Ok(wire) = serde_json::from_slice::<Wire>(&message.payload) else {
			continue;
		};
		let instance = match &wire {
			Wire::Signal { instance, .. }
			| Wire::Presence { instance, .. }
			| Wire::Rooms { instance, .. } => instance,
		};

		if *instance == cluster.instance {
			continue;
		}

		if let Wire::Signal {
			channel,
			from,
			message,
			..
		} = &wire
		{
			deliver_signal(&shared, channel, from.clone(), message.clone());
			continue;
		}

		for channel in cluster.receive(wire) {
			send_presence(&shared, &cluster, &channel);
		}
	}
}

/// Every [`ROOMS`], tells the others every channel's members here, and
/// forgets servers that have gone quiet.
async fn heartbeat(cluster: Weak<Cluster>, shared: Weak<Shared>) {
	let mut tick = tokio::time::interval(ROOMS);
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

	loop {
		tick.tick().await;
		let (Some(cluster), Some(shared)) = (cluster.upgrade(), shared.upgrade()) else {
			return;
		};
		let rooms: BTreeMap<String, Vec<PresenceMember>> = shared
			.rooms
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.iter()
			.map(|(channel, room)| (channel.clone(), room.members.values().cloned().collect()))
			.collect();
		cluster.send(&Wire::Rooms {
			instance: cluster.instance.clone(),
			rooms,
		});

		for channel in cluster.expire() {
			send_presence(&shared, &cluster, &channel);
		}
	}
}

/// Sends a signal from another server to every member of `channel` here.
fn deliver_signal(shared: &Shared, channel: &str, from: Option<String>, message: Value) {
	let senders: Vec<tokio::sync::mpsc::Sender<Outbound>> = {
		let rooms = shared.rooms.lock().unwrap_or_else(PoisonError::into_inner);
		let Some(room) = rooms.get(channel) else {
			return;
		};
		let connections = shared
			.connections
			.lock()
			.unwrap_or_else(PoisonError::into_inner);
		room.members
			.keys()
			.filter_map(|id| connections.get(id).cloned())
			.collect()
	};
	let frame = ServerFrame::Signal {
		channel: channel.to_owned(),
		from,
		message,
	};
	let Some(outbound) = Outbound::frame(&frame, None) else {
		return;
	};

	for sender in senders {
		let _ = sender.try_send(outbound.clone());
	}
}

/// Sends `channel`'s members, here and elsewhere, to its members here.
fn send_presence(shared: &Shared, cluster: &Cluster, channel: &str) {
	let (mut members, senders) = {
		let rooms = shared.rooms.lock().unwrap_or_else(PoisonError::into_inner);
		let Some(room) = rooms.get(channel) else {
			return;
		};
		let connections = shared
			.connections
			.lock()
			.unwrap_or_else(PoisonError::into_inner);
		let senders: Vec<tokio::sync::mpsc::Sender<Outbound>> = room
			.members
			.keys()
			.filter_map(|id| connections.get(id).cloned())
			.collect();
		(room.members.values().cloned().collect::<Vec<_>>(), senders)
	};
	members.extend(cluster.members(channel));
	let frame = ServerFrame::Presence {
		channel: channel.to_owned(),
		members,
	};
	let Some(outbound) = Outbound::frame(&frame, None) else {
		return;
	};

	for sender in senders {
		let _ = sender.try_send(outbound.clone());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn member(id: &str) -> PresenceMember {
		PresenceMember {
			id: id.to_owned(),
			state: Value::Null,
		}
	}

	#[test]
	fn presence_replaces_a_servers_members_and_rooms_drops_the_rest() {
		let mut remote = Remote::default();
		assert!(remote.set("room", "a", vec![member("ada")]));
		assert!(!remote.set("room", "a", vec![member("ada")]));
		assert!(remote.set("room", "b", vec![member("grace")]));
		assert!(remote.set("room", "a", Vec::new()));
		assert_eq!(remote.rooms["room"].len(), 1);
		assert!(remote.set("room", "b", Vec::new()));
		assert!(remote.rooms.is_empty());
	}
}
