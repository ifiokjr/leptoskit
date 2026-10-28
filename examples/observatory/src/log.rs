//! The sky log: a ribbon of nights over a virtualized list of every
//! observation the observatory has made.
//!
//! The log is twenty thousand rows of real widgets' worth, and only the dozen
//! on screen exist. Rows with a note are taller than the rest, and a pressed
//! row opens to show the star's details: the virtualizer starts from those
//! estimates and takes the opened size as a measurement, shifting every row
//! after it.

use std::collections::BTreeSet;
use std::sync::Arc;

use leptoskit::prelude::*;
use leptoskit::virtual_list::Axis;
use leptoskit::virtual_list::Estimate;
use leptoskit::virtual_list::InvalidEstimate;
use leptoskit::virtual_list::VirtualList;
use leptoskit::virtual_list::VirtualOptions;
use leptoskit::virtual_list::Virtualizer;

use crate::catalogue::distance_text;
use crate::catalogue::magnitude_text;
use crate::sky::NIGHTS;
use crate::sky::Observation;
use crate::sky::moon_phase;

/// The log's height, and so its viewport along the scrolling axis.
pub const LOG_HEIGHT: f64 = 400.0;
/// The ribbon's width, its viewport along its axis.
pub const RIBBON_WIDTH: f64 = 560.0;
/// A row without a note.
const ROW: f64 = 52.0;
/// A row whose note takes a second line.
const NOTED_ROW: f64 = 76.0;
/// What an opened row adds for the star's details.
const DETAILS: f64 = 48.0;
/// One night's card in the ribbon.
const CARD: f64 = 64.0;

/// Something the person did to the log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogEvent {
	/// Show one night, or every night.
	SelectNight(Option<u16>),
	/// Open or close the observation at this index of the whole log.
	Toggle(usize),
	/// The log scrolled to this offset.
	Scrolled(f64),
	/// The ribbon scrolled to this offset.
	RibbonScrolled(f64),
	/// Record an observation on the last night, as the telescope does live.
	Record,
}

/// The log's state: every observation, the ones the selected night shows,
/// which are open, and the virtualizers over the log and the ribbon.
#[derive(Debug)]
pub struct SkyLog {
	observations: Vec<Observation>,
	night: Option<u16>,
	/// Indexes into `observations` of the rows shown, in order.
	shown: Vec<usize>,
	/// Indexes into `observations` of the opened rows.
	opened: BTreeSet<usize>,
	virtualizer: Virtualizer,
	ribbon: Virtualizer,
}

impl SkyLog {
	/// The log over `observations`, showing every night.
	pub fn new(observations: Vec<Observation>) -> Self {
		let shown: Vec<usize> = (0..observations.len()).collect();
		let virtualizer = log_virtualizer(&observations, &shown);
		let mut ribbon =
			Virtualizer::new(VirtualOptions::fixed(usize::from(NIGHTS), CARD).with_overscan(3));
		ribbon.set_viewport(RIBBON_WIDTH);
		Self {
			observations,
			night: None,
			shown,
			opened: BTreeSet::new(),
			virtualizer,
			ribbon,
		}
	}

	/// Applies one event.
	pub fn apply(&mut self, event: LogEvent) {
		match event {
			LogEvent::SelectNight(night) => self.select_night(night),
			LogEvent::Toggle(observation) => self.toggle(observation),
			LogEvent::Scrolled(offset) => self.virtualizer.scroll_to(offset),
			LogEvent::RibbonScrolled(offset) => self.ribbon.scroll_to(offset),
			LogEvent::Record => self.record(),
		}
	}

	/// The night shown, or `None` for every night.
	pub fn night(&self) -> Option<u16> {
		self.night
	}

	/// How many observations the log shows.
	pub fn shown_count(&self) -> usize {
		self.shown.len()
	}

	/// Every observation, shown or not.
	pub fn observations(&self) -> &[Observation] {
		&self.observations
	}

	/// Whether the observation at this index of the whole log is open.
	pub fn is_open(&self, observation: usize) -> bool {
		self.opened.contains(&observation)
	}

	/// The first observation in view, which the header reads out as where
	/// the log is.
	pub fn first_in_view(&self) -> Option<&Observation> {
		let items = self.virtualizer.items().ok()?;
		// The window includes overscan; the first row in view is the first
		// one that ends past the offset.
		let offset = self.virtualizer.scroll_offset();
		let item = items.iter().find(|item| item.start + item.size > offset)?;
		self.shown
			.get(item.index)
			.map(|&index| &self.observations[index])
	}

	fn select_night(&mut self, night: Option<u16>) {
		if night == self.night {
			return;
		}
		self.night = night;
		self.shown = self
			.observations
			.iter()
			.enumerate()
			.filter(|(_, observation)| night.is_none_or(|night| observation.night == night))
			.map(|(index, _)| index)
			.collect();
		// A different night is a different list: a fresh virtualizer at its
		// top, with the opened rows that it shows measured again.
		self.virtualizer = log_virtualizer(&self.observations, &self.shown);
		for (position, &index) in self.shown.iter().enumerate() {
			if self.opened.contains(&index) {
				measure(
					&mut self.virtualizer,
					position,
					&self.observations[index],
					true,
				);
			}
		}
	}

	fn toggle(&mut self, observation: usize) {
		if !self.opened.remove(&observation) {
			self.opened.insert(observation);
		}
		if let Some(position) = self.shown.iter().position(|&index| index == observation) {
			let open = self.opened.contains(&observation);
			measure(
				&mut self.virtualizer,
				position,
				&self.observations[observation],
				open,
			);
		}
	}

	fn record(&mut self) {
		let last = self.observations.last().copied();
		let minute = last.map_or(0, |last| last.minute + 3);
		let observation = Observation {
			night: NIGHTS,
			minute,
			star: self.observations.len() % crate::sky::STARS.len(),
			seeing: 4,
			note: (minute % 2 == 0).then_some("Logged live from the dome."),
		};
		self.observations.push(observation);
		let index = self.observations.len() - 1;

		if self.night.is_none_or(|night| night == NIGHTS) {
			self.shown.push(index);
			// The estimate was built for the rows the log had; the new row
			// reports its own size, and every earlier measurement stays.
			self.virtualizer.set_count(self.shown.len());
			measure(
				&mut self.virtualizer,
				self.shown.len() - 1,
				&observation,
				false,
			);
		}
	}
}

/// Hands the virtualizer the real size of the row at `position`.
fn measure(virtualizer: &mut Virtualizer, position: usize, observation: &Observation, open: bool) {
	virtualizer
		.measure(position, row_size(observation, open))
		.expect("a shown position is in range and row sizes are positive");
}

/// The size a row takes: taller with a note, taller again when open.
fn row_size(observation: &Observation, open: bool) -> f64 {
	let base = if observation.note.is_some() {
		NOTED_ROW
	} else {
		ROW
	};
	if open { base + DETAILS } else { base }
}

/// A virtualizer over the shown rows, estimating each from whether it has a
/// note, with a few rows of overscan and room for the column headings.
fn log_virtualizer(observations: &[Observation], shown: &[usize]) -> Virtualizer {
	let noted: Arc<Vec<bool>> = Arc::new(
		shown
			.iter()
			.map(|&index| observations[index].note.is_some())
			.collect(),
	);
	let mut virtualizer = Virtualizer::new(
		VirtualOptions {
			estimate: Estimate::PerItem(Arc::new(move |position| {
				if noted.get(position).copied().unwrap_or(false) {
					NOTED_ROW
				} else {
					ROW
				}
			})),
			padding_start: 8.0,
			padding_end: 8.0,
			..VirtualOptions::fixed(shown.len(), ROW)
		}
		.with_overscan(4),
	);
	virtualizer.set_viewport(LOG_HEIGHT);
	virtualizer
}

/// The ribbon of nights: a horizontal virtualized list of 365 cards, each a
/// toggle that narrows the log to its night.
pub fn ribbon(
	log: &SkyLog,
	on_event: impl Fn(LogEvent) + Clone + Send + Sync + 'static,
) -> Result<Element, InvalidEstimate> {
	let scrolled = on_event.clone();
	let selected = log.night;
	VirtualList(
		&log.ribbon,
		Axis::Horizontal,
		move |offset| scrolled(LogEvent::RibbonScrolled(offset)),
		|item| {
			let night = item.index as u16 + 1;
			let chosen = selected == Some(night);
			let on_event = on_event.clone();
			Pressable(format!("Night {night}"))
				.aria_pressed(chosen)
				.on_press(move || {
					on_event(LogEvent::SelectNight(if chosen {
						None
					} else {
						Some(night)
					}));
				})
				.class(if chosen {
					"flex-1 m-1 items-center justify-center rounded-lg bg-indigo-500 text-white"
				} else {
					"flex-1 m-1 items-center justify-center rounded-lg bg-slate-800 text-slate-300"
				})
				.child(Text(moon_phase(night).to_string()).aria_hidden(true))
				.child(Text(night.to_string()).class("text-xs"))
		},
	)
	.map(|list| {
		list.aria_label("Nights").class(format!(
			"w-[{RIBBON_WIDTH}px] h-[72px] grow-0 rounded-xl bg-slate-900"
		))
	})
}

/// The log itself: one row per observation of the night shown, built only
/// for the rows in view.
pub fn log_list(
	log: &SkyLog,
	on_event: impl Fn(LogEvent) + Clone + Send + Sync + 'static,
) -> Result<Element, InvalidEstimate> {
	let scrolled = on_event.clone();
	VirtualList(
		&log.virtualizer,
		Axis::Vertical,
		move |offset| scrolled(LogEvent::Scrolled(offset)),
		|item| {
			let index = log.shown[item.index];
			let observation = &log.observations[index];
			let open = log.opened.contains(&index);
			let on_event = on_event.clone();
			row(observation, open, move || on_event(LogEvent::Toggle(index)))
		},
	)
	.map(|list| {
		list.aria_label("Observations").class(format!(
			"w-[560px] h-[{LOG_HEIGHT}px] grow-0 rounded-xl bg-slate-900"
		))
	})
}

/// One observation: time, star, and seeing, then its note, then the star's
/// details when open.
fn row(
	observation: &Observation,
	open: bool,
	on_press: impl Fn() + Send + Sync + 'static,
) -> Element {
	let star = observation.star();
	let mut row = Pressable(format!(
		"Night {}, {}, {}, seeing {} of 5",
		observation.night,
		observation.time(),
		star.name,
		observation.seeing
	))
	.aria_expanded(open)
	.on_press(on_press)
	.class("flex-1 gap-1 px-4 py-2 text-slate-100")
	.child(
		View()
			.class("flex-row gap-3 items-center")
			.child(Text(observation.time()).class("w-[48px] text-slate-400"))
			.child(Text(star.name).class("flex-1 font-semibold"))
			.child(Text(observation.seeing_stars()).class("text-amber-300")),
	);

	if let Some(note) = observation.note {
		row = row.child(Text(note).class("text-sm text-slate-400"));
	}
	if open {
		row = row.child(
			Text(format!(
				"{} · class {} · {} · magnitude {}",
				star.constellation,
				star.class,
				distance_text(star.distance),
				magnitude_text(star.magnitude)
			))
			.class("text-sm text-indigo-300"),
		);
	}
	row
}
