//! `use_stream`: a stream's items on screen as they arrive.

use any_spawner::Executor;
use futures::StreamExt;
use leptos::prelude::*;

use super::FnError;
use super::live::StreamEvent;
use super::live::client_if_any;
use crate::StreamOperation;
use crate::live::EndReason;

/// How many items the app may hold before the server waits for it to take
/// some: the stream's window.
const WINDOW: u32 = 64;

/// Where a stream on screen is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamState {
	/// Opening, or waiting for the live connection.
	Opening,
	/// Items are arriving.
	Open,
	/// It sent its last item.
	Done,
	/// It failed, or never opened.
	Failed(FnError),
	/// The connection went before it finished, and it can't resume.
	Interrupted,
}

impl StreamState {
	/// Whether nothing more will arrive.
	#[must_use]
	pub fn is_finished(&self) -> bool {
		matches!(self, Self::Done | Self::Failed(_) | Self::Interrupted)
	}
}

/// What a stream's screen sees: the items so far, and where the stream is.
pub struct StreamHandle<T: Send + Sync + 'static> {
	items: RwSignal<Vec<T>>,
	state: RwSignal<StreamState>,
}

impl<T: Send + Sync + 'static> Clone for StreamHandle<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T: Send + Sync + 'static> Copy for StreamHandle<T> {}

impl<T: Clone + Send + Sync + 'static> StreamHandle<T> {
	/// Every item so far, oldest first.
	pub fn items(&self) -> ReadSignal<Vec<T>> {
		self.items.read_only()
	}

	/// Where the stream is.
	pub fn state(&self) -> ReadSignal<StreamState> {
		self.state.read_only()
	}

	/// Forgets the items so far; new ones keep arriving.
	pub fn clear(&self) {
		self.items.set(Vec::new());
	}
}

/// Opens a stream while the component is mounted, again when its arguments
/// change, and cancels it when it unmounts.
///
/// ```ignore
/// let answer = rpc::use_stream(move || Answer { prompt: prompt.get() });
/// let text = move || answer.items().get().concat();
/// let thinking = move || !answer.state().get().is_finished();
/// ```
///
/// Items arrive as fast as the screen takes them, and the server reads no
/// further ahead than that. A `resumable` stream opens again from its last
/// item after a reconnect; any other ends as
/// [`Interrupted`](StreamState::Interrupted). Without a live connection,
/// as while the server renders, it stays [`Opening`](StreamState::Opening).
#[track_caller]
pub fn use_stream<S>(args: impl Fn() -> S + Send + Sync + 'static) -> StreamHandle<S::Item>
where
	S: StreamOperation,
	S::Item: Clone + Send + Sync,
{
	let items = RwSignal::new(Vec::new());
	let state = RwSignal::new(StreamState::Opening);
	let open = StoredValue::new(None::<futures::channel::oneshot::Sender<()>>);

	Effect::new_isomorphic(move |_| {
		let args = args();
		let Some(live) = client_if_any() else {
			return;
		};
		// Closing the previous one's channel ends its task, which drops
		// (and so cancels) its stream.
		open.set_value(None);
		let Ok(value) = serde_json::to_value(&args) else {
			state.set(StreamState::Failed(FnError::Invalid(
				"the stream's arguments aren't JSON".to_owned(),
			)));
			return;
		};

		items.set(Vec::new());
		state.set(StreamState::Opening);
		let stream = live.stream(S::NAME, value, WINDOW, S::RESUMABLE);
		let (stop, stopped) = futures::channel::oneshot::channel();
		open.set_value(Some(stop));

		Executor::spawn_local(follow::<S::Item>(stream, stopped, items, state));
	});

	on_cleanup(move || {
		let _ = open.try_set_value(None);
	});

	StreamHandle { items, state }
}

/// Puts each of the stream's items on screen until it ends, or the screen
/// stops it.
async fn follow<T: serde::de::DeserializeOwned + Send + Sync + 'static>(
	mut stream: super::live::LiveStream,
	mut stopped: futures::channel::oneshot::Receiver<()>,
	items: RwSignal<Vec<T>>,
	state: RwSignal<StreamState>,
) {
	loop {
		let event = match futures::future::select(stream.next(), &mut stopped).await {
			futures::future::Either::Left((event, _)) => event,
			futures::future::Either::Right(_) => return,
		};

		let shown = match event {
			Some(StreamEvent::Item(value)) => {
				if let Ok(item) = serde_json::from_value::<T>(value) {
					let _ = items.try_update(|items| items.push(item));
				}

				state.try_update(opened).is_some()
			}
			Some(StreamEvent::End(reason)) => {
				let _ = state.try_set(match reason {
					EndReason::Done => StreamState::Done,
					EndReason::Interrupted => StreamState::Interrupted,
					EndReason::Failed(error) => StreamState::Failed(super::live::fn_error(error)),
				});
				false
			}
			None => false,
		};

		if !shown {
			return;
		}
	}
}

/// A first item opens the stream.
fn opened(state: &mut StreamState) {
	if *state == StreamState::Opening {
		*state = StreamState::Open;
	}
}
