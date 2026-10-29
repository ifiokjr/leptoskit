//! DOM-owned players survive reconciliation and drop with their canvas state.

use leptos::tachys::html::attribute::Attribute;
use leptos::tachys::html::attribute::NamedAttributeKey;
use leptos::tachys::html::attribute::NextAttribute;
use leptoskit_rive::Playback;

#[derive(Clone)]
pub(super) struct Binding {
	pub content: Playback,
	pub running: bool,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod client {
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::rc::Weak;

	use leptos::prelude::*;
	use leptoskit_rive::Error;
	use leptoskit_rive::Playback;
	use leptoskit_rive::web::Options;
	use leptoskit_rive::web::Player;
	use leptoskit_rive::web::Runtime;
	use wasm_bindgen::JsCast;

	use super::Binding;

	type RuntimeContext = StoredValue<Runtime, LocalStorage>;

	/// Supplies the shared Rive canvas element's app-owned browser runtime.
	/// Call in the client root's Leptos owner before rendering shared elements.
	/// The app pins and serves the runtime JS and Wasm; no scripts are
	/// injected.
	pub fn provide_rive_runtime(runtime: Runtime) {
		provide_context(RuntimeContext::new_local(runtime));
	}

	pub struct State(Rc<RefCell<Live>>);

	#[derive(Clone, Copy, PartialEq, Eq)]
	enum Phase {
		Loading,
		Ready,
		Failed,
		Closed,
	}

	struct Live {
		canvas: web_sys::HtmlCanvasElement,
		runtime: Option<Runtime>,
		content: Playback,
		running: bool,
		applied: Option<Playback>,
		player: Option<Player>,
		generation: u64,
		phase: Phase,
	}

	impl Drop for State {
		fn drop(&mut self) {
			let mut live = self.0.borrow_mut();
			live.phase = Phase::Closed;
			live.player.take();
		}
	}

	impl Live {
		fn fail(&mut self, error: &Error) {
			self.player.take();
			self.phase = Phase::Failed;
			let message = error.to_string();
			web_sys::console::error_1(&message.clone().into());
			if let Err(error) = self.canvas.set_attribute("data-lk-rive-error", &message) {
				web_sys::console::error_1(&error);
			}
		}

		fn apply(&mut self) -> Result<(), Error> {
			let Some(player) = self.player.as_ref().filter(|_| self.phase == Phase::Ready) else {
				return Ok(());
			};
			let changed = self.content.inputs().iter().filter(|(name, update)| {
				self.applied
					.as_ref()
					.and_then(|old| old.inputs().get(*name))
					!= Some(*update)
			});
			for (name, update) in changed.clone() {
				player.validate_input(name, update.value)?;
			}
			for (name, update) in changed {
				player.set_input(name, update.value)?;
			}
			if self.running {
				player.play()?;
			} else {
				player.pause()?;
			}
			self.applied = Some(self.content.clone());
			Ok(())
		}
	}

	impl State {
		pub(super) fn new(binding: Binding, element: &web_sys::Element) -> Self {
			let runtime = use_context::<RuntimeContext>().map(|value| value.get_value());
			let state = Self(Rc::new(RefCell::new(Live {
				canvas: element
					.clone()
					.dyn_into()
					.expect("Rive binding belongs to a canvas"),
				runtime,
				content: binding.content,
				running: binding.running,
				applied: None,
				player: None,
				generation: 0,
				phase: Phase::Loading,
			})));
			state.reload();
			state
		}

		pub(super) fn update(&mut self, binding: Binding) {
			let mut live = self.0.borrow_mut();
			let reload = !live.content.same_scene(&binding.content);
			live.content = binding.content;
			live.running = binding.running;
			if reload {
				drop(live);
				self.reload();
			} else if let Err(error) = live.apply() {
				live.fail(&error);
			}
		}

		fn reload(&self) {
			let mut live = self.0.borrow_mut();
			live.player.take();
			live.applied = None;
			live.phase = Phase::Loading;
			if let Err(error) = live.canvas.remove_attribute("data-lk-rive-error") {
				web_sys::console::error_1(&error);
				live.fail(&Error::Runtime(
					"could not clear the previous Rive canvas diagnostic".into(),
				));
				return;
			}
			live.generation += 1;
			let generation = live.generation;
			let weak = Rc::downgrade(&self.0);
			// Attribute build precedes DOM insertion. Mount once that
			// synchronous insertion has finished; removal before then cancels
			// this generation.
			queue_microtask(move || Self::mount(weak, generation));
		}

		fn mount(weak: Weak<RefCell<Live>>, generation: u64) {
			let Some(shared) = weak.upgrade() else {
				return;
			};
			let mut live = shared.borrow_mut();
			if live.phase != Phase::Loading || live.generation != generation {
				return;
			}
			let result = live
				.runtime
				.as_ref()
				.ok_or_else(|| {
					Error::Runtime(
						"call provide_rive_runtime before rendering shared Rive elements in the \
						 browser"
							.into(),
					)
				})
				.and_then(|runtime| {
					Player::new(
						runtime,
						&live.canvas,
						live.content.asset(),
						&Options {
							artboard: live.content.artboard_name().cloned(),
							scene: live.content.selected_scene().clone(),
							autoplay: false,
						},
					)
				});
			let player = match result {
				Ok(player) => player,
				Err(error) => {
					live.fail(&error);
					return;
				}
			};
			let ready = player.ready();
			live.player = Some(player);
			drop(live);
			leptos::task::spawn_local(async move {
				let result = ready.await;
				let Some(shared) = weak.upgrade() else {
					return;
				};
				let mut live = shared.borrow_mut();
				if live.phase != Phase::Loading || live.generation != generation {
					return;
				}
				live.phase = Phase::Ready;
				if let Err(error) = result.and_then(|()| live.apply()) {
					live.fail(&error);
				}
			});
		}
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use client::provide_rive_runtime;

impl Attribute for Binding {
	type AsyncOutput = Self;
	type Cloneable = Self;
	type CloneableOwned = Self;
	#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
	type State = client::State;
	#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
	type State = ();

	const MIN_LENGTH: usize = 0;

	fn html_len(&self) -> usize {
		0
	}

	fn to_html(
		self,
		_buf: &mut String,
		_class: &mut String,
		_style: &mut String,
		_inner: &mut String,
	) {
	}

	#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
	fn hydrate<const FROM_SERVER: bool>(self, element: &web_sys::Element) -> Self::State {
		self.build(element)
	}

	#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
	fn hydrate<const FROM_SERVER: bool>(self, element: &web_sys::Element) -> Self::State {
		self.build(element);
	}

	fn build(self, element: &web_sys::Element) -> Self::State {
		#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
		{
			client::State::new(self, element)
		}
		#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
		{
			let _ = (self.content, self.running, element);
		}
	}

	fn rebuild(self, state: &mut Self::State) {
		#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
		state.update(self);
		#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
		let _ = (self, state);
	}

	fn into_cloneable(self) -> Self {
		self
	}

	fn into_cloneable_owned(self) -> Self {
		self
	}

	fn dry_resolve(&mut self) {}

	fn resolve(self) -> impl Future<Output = Self> + Send {
		std::future::ready(self)
	}

	fn keys(&self) -> Vec<NamedAttributeKey> {
		Vec::new()
	}
}

impl NextAttribute for Binding {
	type Output<NewAttr: Attribute> = (Self, NewAttr);

	fn add_any_attr<NewAttr: Attribute>(self, next: NewAttr) -> Self::Output<NewAttr> {
		(self, next)
	}
}
