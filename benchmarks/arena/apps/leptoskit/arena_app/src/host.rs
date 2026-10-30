//! What the scenarios need from the platform: the launch scenario, a
//! monotonic clock, a frame clock, the system log, and the echo module.
//!
//! Native builds reach all of them through runtime plugins, because the
//! WebAssembly guest has no clock, timer, frame callback, log, or argument
//! access of its own. The browser build calls the web platform directly.

pub use arena_bridge::Frame;

/// The destination name used in reports.
pub const PLATFORM: &str = if cfg!(feature = "ios") {
	"ios"
} else if cfg!(feature = "android") {
	"android"
} else if cfg!(feature = "macos") {
	"macos"
} else if cfg!(feature = "windows") {
	"windows"
} else if cfg!(feature = "linux") {
	"linux"
} else {
	"web"
};

#[cfg(all(target_arch = "wasm32", feature = "native"))]
pub use native::Host;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use web::Host;

#[cfg(all(target_arch = "wasm32", feature = "native"))]
mod native {
	use arena_bridge::guest::Arena;
	use arena_clock::guest::Clock;

	use super::Frame;

	/// The native plugins, resolved once.
	#[derive(Debug, Clone, Copy)]
	pub struct Host {
		arena: Arena,
		clock: Clock,
	}

	impl Host {
		pub fn new() -> Result<Self, String> {
			Ok(Self {
				arena: Arena::new().map_err(|error| format!("arena plugin: {error:?}"))?,
				clock: Clock::new().map_err(|error| format!("arena-clock plugin: {error:?}"))?,
			})
		}

		pub async fn scenario(self) -> Result<Option<String>, String> {
			self.arena
				.scenario()
				.await
				.map_err(|error| format!("scenario: {error:?}"))
		}

		/// Milliseconds on the host's monotonic clock.
		pub async fn now_ms(self) -> Result<f64, String> {
			let nanos = self
				.clock
				.now_ns()
				.await
				.map_err(|error| format!("clock: {error:?}"))?;
			Ok(nanos as f64 / 1e6)
		}

		/// leptoskit's frame clock: the host's display link resumes this, and
		/// the guest's render is committed in the same callback.
		pub async fn next_frame(self) -> Result<Frame, String> {
			let frame = leptoskit::guest::next_frame()
				.await
				.map_err(|error| format!("frame: {error:?}"))?;
			Ok(Frame {
				timestamp_ns: frame.timestamp_ns,
				period_ns: frame.period_ns,
			})
		}

		pub async fn report(self, lines: &str) -> Result<(), String> {
			self.arena
				.report(lines)
				.await
				.map_err(|error| format!("report: {error:?}"))
		}

		/// The `ArenaEcho` module.
		pub fn echo(self) -> Result<Arena, String> {
			Ok(self.arena)
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;

	use super::Frame;

	/// The browser's own services.
	#[derive(Debug, Clone, Copy)]
	pub struct Host;

	impl Host {
		pub fn new() -> Result<Self, String> {
			Ok(Self)
		}

		/// `?scenario=<id>`.
		pub async fn scenario(self) -> Result<Option<String>, String> {
			let search = window()?.location().search().map_err(describe)?;
			let query = web_sys::UrlSearchParams::new_with_str(&search).map_err(describe)?;
			Ok(query
				.get("scenario")
				.filter(|scenario| !scenario.is_empty()))
		}

		/// `performance.now()`.
		pub async fn now_ms(self) -> Result<f64, String> {
			Ok(window()?.performance().ok_or("no performance clock")?.now())
		}

		/// The next `requestAnimationFrame` callback. The browser exposes no
		/// refresh period, so `period_ns` is zero.
		pub async fn next_frame(self) -> Result<Frame, String> {
			let window = window()?;
			let promise = js_sys::Promise::new(&mut |resolve, _reject| {
				// A failed request leaves the promise pending; the browser
				// only fails it for a detached document.
				let _ = window.request_animation_frame(&resolve);
			});
			let timestamp = wasm_bindgen_futures::JsFuture::from(promise)
				.await
				.map_err(describe)?
				.as_f64()
				.ok_or("requestAnimationFrame passed no timestamp")?;
			Ok(Frame {
				timestamp_ns: (timestamp * 1e6) as u64,
				period_ns: 0,
			})
		}

		/// `console.log`, one call per line.
		pub async fn report(self, lines: &str) -> Result<(), String> {
			for line in lines.split('\n') {
				web_sys::console::log_1(&JsValue::from_str(line));
			}
			Ok(())
		}

		/// There is no native layer to call.
		pub fn echo(self) -> Result<std::convert::Infallible, String> {
			Err("no native layer on the web".to_owned())
		}
	}

	fn window() -> Result<web_sys::Window, String> {
		web_sys::window().ok_or_else(|| "no window".to_owned())
	}

	fn describe(error: JsValue) -> String {
		error.dyn_ref::<js_sys::Error>().map_or_else(
			|| format!("{error:?}"),
			|error| String::from(error.message()),
		)
	}
}
