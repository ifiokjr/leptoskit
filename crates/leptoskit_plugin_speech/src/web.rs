//! Browser speech synthesis. Instances share one bounded active utterance and
//! never cancel speech queued by unrelated code when they own no utterance.

use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Weak;

use futures::channel::oneshot;
use gloo_events::EventListener;
use wasm_bindgen::JsCast;
use web_sys::SpeechSynthesis;
use web_sys::SpeechSynthesisUtterance;
use web_sys::SpeechSynthesisVoice;

use super::Error;
use super::Quality;
use super::Utterance;
use super::Voice;

type Completion = oneshot::Sender<Result<(), Error>>;

/// Converts a thousandths value into the browser's 0..=1 fraction.
fn fraction(value: u32) -> Result<f32, Error> {
	let thousandths = u16::try_from(value).map_err(|_| Error::InvalidArgument)?;

	Ok(f32::from(thousandths) / 1000.0)
}

/// Maps the browser's speech error code onto the plugin's error vocabulary.
fn failure_code(event: &web_sys::Event) -> Error {
	let Ok(code) = js_sys::Reflect::get(event, &wasm_bindgen::JsValue::from_str("error")) else {
		return Error::Native;
	};

	match code.as_string().as_deref() {
		Some("canceled" | "interrupted") => Error::Cancelled,
		Some("not-allowed") => Error::PermissionDenied,
		Some("voice-unavailable" | "language-unavailable" | "synthesis-unavailable") => {
			Error::Unavailable
		}
		Some("text-too-long") => Error::TooLarge,
		Some("invalid-argument") => Error::InvalidArgument,
		_ => Error::Native,
	}
}
struct Active {
	token: u64,
	completion: Completion,
}
struct State {
	synthesis: SpeechSynthesis,
	active: RefCell<Option<Active>>,
	next: RefCell<u64>,
	closed: RefCell<bool>,
}
thread_local! { static SHARED: RefCell<Weak<State>> = const { RefCell::new(Weak::new()) }; }
impl State {
	/// Takes the active utterance back, but only when this token owns it.
	fn take(&self, token: u64) -> Option<Active> {
		let mut active = self.active.borrow_mut();

		if active.as_ref().is_none_or(|active| active.token != token) {
			return None;
		}

		active.take()
	}

	fn finish(&self, token: u64, result: Result<(), Error>) {
		if let Some(active) = self.take(token) {
			let _ = active.completion.send(result);
		}
	}

	fn cancel(&self, token: u64) {
		if let Some(active) = self.take(token) {
			self.synthesis.cancel();
			let _ = active.completion.send(Err(Error::Cancelled));
		}
	}
}
struct Guard {
	state: Rc<State>,
	token: u64,
}
impl Drop for Guard {
	fn drop(&mut self) {
		self.state.cancel(self.token);
	}
}

/// A handle to the document's speech service. Explicit close cancels its active
/// utterance and closes shared handles; a later `new` creates fresh handles.
#[derive(Clone)]
pub struct Speech {
	state: Rc<State>,
}
impl Speech {
	/// Resolves the browser speech service. No audio starts here.
	pub fn new() -> Result<Self, Error> {
		SHARED.with(|shared| {
			if let Some(existing) = Self::reusable(shared) {
				return Ok(Self { state: existing });
			}

			let synthesis = web_sys::window()
				.ok_or(Error::Unavailable)?
				.speech_synthesis()
				.map_err(|_| Error::Unavailable)?;
			let state = Rc::new(State {
				synthesis,
				active: RefCell::new(None),
				next: RefCell::new(0),
				closed: RefCell::new(false),
			});
			*shared.borrow_mut() = Rc::downgrade(&state);
			Ok(Self { state })
		})
	}

	/// The shared handles a later `new` may reuse, when they are still open.
	fn reusable(shared: &RefCell<Weak<State>>) -> Option<Rc<State>> {
		let state = shared.borrow().upgrade()?;

		if *state.closed.borrow() {
			return None;
		}

		Some(state)
	}

	fn open(&self) -> Result<(), Error> {
		if *self.state.closed.borrow() {
			Err(Error::Cancelled)
		} else {
			Ok(())
		}
	}

	/// Returns the browser's currently loaded voices. Some browsers initially
	/// return an empty list; re-query after their `voiceschanged` event.
	pub fn voices(&self) -> Result<Vec<Voice>, Error> {
		self.open()?;
		let voices = self.state.synthesis.get_voices();

		if voices.length() > 1024 {
			return Err(Error::TooLarge);
		}

		voices
			.iter()
			.map(|value| {
				let voice = value
					.dyn_into::<SpeechSynthesisVoice>()
					.map_err(|_| Error::Native)?;
				Ok(Voice {
					id: voice.voice_uri(),
					name: voice.name(),
					language: voice.lang(),
					quality: Quality::Unknown,
					local: Some(voice.local_service()),
				})
			})
			.collect()
	}

	/// Speaks one utterance until end/error/stop. Dropping this future stops
	/// only its own active utterance. Call from a user interaction where
	/// browsers require it.
	pub async fn speak(&self, request: &Utterance) -> Result<(), Error> {
		self.open()?;
		self.require_idle()?;
		let utterance = self.utterance(request)?;
		let token = self.next_token()?;
		let (completion, receiver) = oneshot::channel();
		*self.state.active.borrow_mut() = Some(Active { token, completion });
		let guard = Guard {
			state: Rc::clone(&self.state),
			token,
		};
		let ended = self.end_listener(&utterance, token);
		let failed = self.error_listener(&utterance, token);

		self.state.synthesis.speak(&utterance);

		let result = receiver.await.map_err(|_| Error::Cancelled)?;
		drop((ended, failed, guard, utterance));
		result
	}

	/// Rejects a second utterance while the first one still owns the service.
	fn require_idle(&self) -> Result<(), Error> {
		if self.state.active.borrow().is_some() {
			return Err(Error::Busy);
		}

		if self.state.synthesis.speaking() || self.state.synthesis.pending() {
			return Err(Error::Busy);
		}

		Ok(())
	}

	/// Builds the browser utterance the request describes.
	fn utterance(&self, request: &Utterance) -> Result<SpeechSynthesisUtterance, Error> {
		let utterance = SpeechSynthesisUtterance::new_with_text(request.text())
			.map_err(|_| Error::Unavailable)?;
		utterance.set_rate(fraction(request.rate())?);
		utterance.set_pitch(fraction(request.pitch())?);
		utterance.set_volume(fraction(request.volume())?);

		if let Some(language) = request.language() {
			utterance.set_lang(language);
		}

		if let Some(id) = request.voice() {
			utterance.set_voice(Some(&self.voice(id)?));
		}

		Ok(utterance)
	}

	/// Finds the installed voice the request named.
	fn voice(&self, id: &str) -> Result<SpeechSynthesisVoice, Error> {
		for value in self.state.synthesis.get_voices().iter() {
			let voice = value
				.dyn_into::<SpeechSynthesisVoice>()
				.map_err(|_| Error::Native)?;

			if voice.voice_uri() == id {
				return Ok(voice);
			}
		}

		Err(Error::Unavailable)
	}

	/// Allocates the token that attributes browser callbacks to this utterance.
	fn next_token(&self) -> Result<u64, Error> {
		let token = self
			.state
			.next
			.borrow()
			.checked_add(1)
			.ok_or(Error::Exhausted)?;
		*self.state.next.borrow_mut() = token;

		Ok(token)
	}

	/// Answers the wait when the browser reports the utterance finished.
	fn end_listener(&self, utterance: &SpeechSynthesisUtterance, token: u64) -> EventListener {
		let state = Rc::clone(&self.state);

		EventListener::new(utterance, "end", move |_| state.finish(token, Ok(())))
	}

	/// Answers the wait with the error the browser reports.
	fn error_listener(&self, utterance: &SpeechSynthesisUtterance, token: u64) -> EventListener {
		let state = Rc::clone(&self.state);

		EventListener::new(utterance, "error", move |event| {
			state.finish(token, Err(failure_code(event)));
		})
	}

	/// Stops only the utterance owned by these shared handles.
	pub fn stop(&self) -> Result<(), Error> {
		self.open()?;
		let token = self
			.state
			.active
			.borrow()
			.as_ref()
			.map(|active| active.token);

		if let Some(token) = token {
			self.state.cancel(token);
		}

		Ok(())
	}

	/// Reports owned active work, not unrelated speech on the page.
	pub fn is_speaking(&self) -> bool {
		self.state.active.borrow().is_some()
	}

	/// Cancels owned work and prevents subsequent use of these handles.
	pub fn close(&self) {
		*self.state.closed.borrow_mut() = true;
		let token = self
			.state
			.active
			.borrow()
			.as_ref()
			.map(|active| active.token);

		if let Some(token) = token {
			self.state.cancel(token);
		}
	}
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::*;

	use super::*;
	wasm_bindgen_test_configure!(run_in_browser);
	#[wasm_bindgen_test]
	async fn muted_stop_drop_and_close_release_bounded_work() {
		let speech = Speech::new().unwrap();
		let request = Utterance::new("Muted cancellation test")
			.unwrap()
			.with_audio(1000, 1000, 0)
			.unwrap();
		let mut speaking = Box::pin(speech.speak(&request));
		let initial = futures::poll!(&mut speaking);
		assert!(initial.is_pending(), "initial speech state: {initial:?}");
		assert!(speech.is_speaking());
		assert_eq!(speech.speak(&request).await, Err(Error::Busy));
		speech.stop().unwrap();

		assert_eq!(speaking.await, Err(Error::Cancelled));
		assert!(!speech.is_speaking());
		let mut dropped = Box::pin(speech.speak(&request));
		assert!(futures::poll!(&mut dropped).is_pending());
		drop(dropped);
		assert!(!speech.is_speaking());
		speech.close();
		assert_eq!(speech.stop(), Err(Error::Cancelled));

		assert_eq!(speech.speak(&request).await, Err(Error::Cancelled));
	}
}
