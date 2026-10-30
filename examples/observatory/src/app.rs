//! The observatory's state and its one screen.

use std::sync::Arc;

use leptoskit::prelude::*;
use leptoskit::table::TableState;
use parking_lot::RwLock;

use crate::catalogue;
use crate::log;
use crate::log::LogEvent;
use crate::log::SkyLog;
use crate::sky;

/// The desktop viewport the screen is laid out for.
pub const VIEWPORT: (f32, f32) = (1280.0, 820.0);

/// Everything the screen shows that the person can change.
#[derive(Debug)]
struct State {
	catalogue: TableState,
	log: SkyLog,
}

/// The observatory: a star catalogue beside a year's sky log.
///
/// It is a handle — cloning it shares the state — so the screen's event
/// handlers can hold it and a host can render it again after each event:
///
/// ```
/// use observatory_example::Observatory;
///
/// let observatory = Observatory::new();
/// let screen = observatory.render();
/// assert_eq!(screen.children.len(), 2);
/// assert!(observatory.log_count() > 14_000);
/// ```
#[derive(Debug, Clone)]
pub struct Observatory {
	state: Arc<RwLock<State>>,
}

impl Default for Observatory {
	fn default() -> Self {
		Self::new()
	}
}

impl Observatory {
	/// The observatory over the generated sky.
	pub fn new() -> Self {
		Self {
			state: Arc::new(RwLock::new(State {
				catalogue: catalogue::initial_state(),
				log: SkyLog::new(sky::observations()),
			})),
		}
	}

	/// The catalogue's current table state.
	pub fn catalogue_state(&self) -> TableState {
		self.state.read().catalogue.clone()
	}

	/// How many observations the log shows for the selected night.
	pub fn log_count(&self) -> usize {
		self.state.read().log.shown_count()
	}

	/// The night the log shows, or `None` for every night.
	pub fn night(&self) -> Option<u16> {
		self.state.read().log.night()
	}

	/// Applies an event to the log, as its controls do.
	pub fn log_event(&self, event: LogEvent) {
		self.state.write().log.apply(event);
	}

	/// The whole screen for the current state.
	///
	/// # Panics
	/// Panics if a log row's size estimate is not positive, which the fixed
	/// row heights rule out.
	pub fn render(&self) -> Element {
		let state = self.state.read();
		let on_catalogue = {
			let observatory = self.clone();
			move |next| observatory.state.write().catalogue = next
		};
		let on_log = {
			let observatory = self.clone();
			move |event| observatory.log_event(event)
		};

		view! {
			<View class="flex-col gap-4 p-6 w-full h-full bg-slate-950">
				{header()}
				// Top-aligned, so the catalogue keeps its own height beside
				// the taller log.
				<View class="flex-row items-start gap-6">
					{catalogue::catalogue(&state.catalogue, on_catalogue)}
					{sky_log(&state.log, on_log)}
				</View>
			</View>
		}
	}
}

fn header() -> Element {
	view! {
		<View class="gap-1">
			<Heading aria_level=1 class="text-[34px] font-bold text-slate-100">
				"Observatory"
			</Heading>
			<Text class="text-slate-400">
				"Forty-four bright stars, and every look at them in a year of nights."
			</Text>
		</View>
	}
}

/// The log section: where the log is, the ribbon of nights, the log, and
/// the live record button.
fn sky_log(log: &SkyLog, on_event: impl Fn(LogEvent) + Clone + Send + Sync + 'static) -> Element {
	let count = grouped(log.shown_count());
	let showing = match log.night() {
		Some(night) => format!("Night {night} · {count} observations"),
		None => format!("Every night · {count} observations"),
	};
	let position = log.first_in_view().map_or_else(
		|| "Nothing logged".to_owned(),
		|observation| format!("At night {}, {}", observation.night, observation.time()),
	);

	let select_all = {
		let on_event = on_event.clone();
		move || on_event(LogEvent::SelectNight(None))
	};
	let record = {
		let on_event = on_event.clone();
		move || on_event(LogEvent::Record)
	};

	view! {
		<View class="gap-3 w-[560px]">
			<Heading aria_level=2 class="text-2xl font-bold text-slate-100">"Sky log"</Heading>
			<View class="flex-row gap-3 items-center">
				<Text class="flex-1 text-slate-300">{showing}</Text>
				<Button enabled=log.night().is_some() on:press=select_all>"All nights"</Button>
				<Button on:press=record>"Record an observation"</Button>
			</View>
			{log::ribbon(log, on_event.clone()).expect("night cards have a fixed size")}
			<Text class="text-sm text-slate-400">{position}</Text>
			{log::log_list(log, on_event).expect("row sizes are fixed and positive")}
		</View>
	}
}

/// A count with thousands separated, as `20,151`.
pub fn grouped(count: usize) -> String {
	let digits = count.to_string();
	let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index) % 3 == 0 {
			grouped.push(',');
		}
		grouped.push(digit);
	}
	grouped
}
