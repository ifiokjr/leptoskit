//! The departures board: queries over the harbour's server functions, a
//! booking mutation that updates the board before the server answers, the
//! log as an infinite query, and hooks for the day, the search, the clock,
//! the departed sailings, and the pinned route.

use std::convert::Infallible;
use std::time::Duration;

use leptoskit::hooks::Clock;
use leptoskit::hooks::MemoryStorage;
use leptoskit::hooks::UseCycleListReturn;
use leptoskit::hooks::UseStorageReturn;
use leptoskit::hooks::UseToggleReturn;
use leptoskit::hooks::signal_debounced;
use leptoskit::hooks::use_cycle_list;
use leptoskit::hooks::use_interval_fn;
use leptoskit::hooks::use_storage;
use leptoskit::hooks::use_toggle;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::query::InfiniteQueryHandle;
use leptoskit::query::InfiniteQueryScope;
use leptoskit::query::MutationHandle;
use leptoskit::query::MutationScope;
use leptoskit::query::NetworkMode;
use leptoskit::query::QueryClient;
use leptoskit::query::QueryDefaults;
use leptoskit::query::QueryFilter;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::QueryScope;
use leptoskit::query::RefetchOn;
use leptoskit::query::Retry;
use leptoskit::query::use_infinite_query;
use leptoskit::query::use_is_fetching;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;
use leptoskit::rpc::FnError;
use reactive_graph::owner::Owner;
use reactive_graph::signal::RwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Set;
use reactive_graph::traits::Update;
use reactive_graph::wrappers::read::Signal;

use crate::api;
use crate::harbor::DAYS;
use crate::harbor::LogPage;
use crate::harbor::Sailing;
use crate::harbor::SailingStatus;

/// The board's viewport.
pub const VIEWPORT: (f32, f32) = (900.0, 760.0);

/// How long the search waits for typing to pause before it filters.
pub const SEARCH_DELAY: Duration = Duration::from_millis(300);

/// How often the board reads the day's sailings again while it is open.
pub const REFRESH: Duration = Duration::from_secs(30);

/// Where the harbour clock starts: 06:00:00.
const OPENING: Duration = Duration::from_hours(6);

/// A seat booking: which day's board shows the sailing, and which sailing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Booking {
	/// The day whose cached board the booking updates.
	pub day: u16,
	/// The sailing to book.
	pub sailing: u32,
}

/// One day's sailings, read through the `sailings` server function and kept
/// fresh for half a minute.
pub fn sailings() -> QueryScope<u16, Vec<Sailing>, FnError> {
	QueryScope::new(|day| async move { api::sailings(day).await.map_err(FnError::from) })
		.with_stale_time(Duration::from_secs(30))
		.with_retry(Retry::never())
}

/// The harbour master's log, a page at a time, newest first.
pub fn log() -> InfiniteQueryScope<(), u32, LogPage, FnError> {
	InfiniteQueryScope::new(
		|context| {
			async move {
				api::harbor_log(context.page_param)
					.await
					.map_err(FnError::from)
			}
		},
		0,
	)
	.with_next_page_param(|context| context.page.next)
}

/// Books a seat. The board drops the seat count at once and puts it back if
/// the server refuses; either way it then reads the day again.
pub fn booking() -> MutationScope<Booking, Sailing, FnError, Option<Vec<Sailing>>> {
	MutationScope::new(|booking: Booking| {
		async move { api::book_seat(booking.sailing).await.map_err(FnError::from) }
	})
	.with_on_mutate_sync(|args| {
		let day = args.variables.day;
		// A read already on its way would land on top of the optimistic
		// board, so stop it first.
		args.client
			.cancel_queries(QueryFilter::key(&sailings(), &day));
		let previous = args.client.get_query_data(&sailings(), &day);
		args.client.update_query_data(&sailings(), day, |board| {
			let mut board = board?;
			if let Some(sailing) = board
				.iter_mut()
				.find(|sailing| sailing.id == args.variables.sailing)
			{
				sailing.seats_left = sailing.seats_left.saturating_sub(1);
			}
			Some(board)
		});
		previous
	})
	.with_on_error(|args| {
		match args.context {
			Some(previous) => {
				args.client
					.set_query_data(&sailings(), args.variables.day, previous);
			}
			None => {
				args.client
					.remove_queries(QueryFilter::key(&sailings(), &args.variables.day));
			}
		}
	})
	.with_on_settled_async(|args| {
		async move {
			args.client
				.invalidate_query(&sailings(), &args.variables.day)
				.await;
		}
	})
}

/// The board: its queries, its mutation, its hooks, and the reactive owner
/// they all live in.
///
/// Create it inside a running async executor; the queries fetch on it. The
/// host advances [`Board::advance`] from its own timer, so the clock and the
/// debounced search move at the host's pace — or a test's.
#[derive(Clone)]
pub struct Board {
	owner: Owner,
	client: QueryClient,
	clock: RwSignal<Duration>,
	days: UseCycleListReturn<u16>,
	search: RwSignal<String>,
	searched: Signal<String>,
	departed: UseToggleReturn,
	pinned: UseStorageReturn<Option<String>, Infallible>,
	departures: QueryHandle<Vec<Sailing>, FnError>,
	log: InfiniteQueryHandle<LogPage, u32, FnError>,
	book: MutationHandle<Booking, Sailing, FnError, Option<Vec<Sailing>>>,
}

impl Board {
	/// A board over `storage`, where it keeps the pinned route.
	pub fn new(storage: MemoryStorage<Option<String>>) -> Self {
		let owner = Owner::new();
		owner.with(|| {
			let client = QueryClient::new().with_defaults(
				QueryDefaults::new()
					.with_refetch_on_window_focus(RefetchOn::Never)
					.with_refetch_on_reconnect(RefetchOn::Never)
					.with_network_mode(NetworkMode::Always),
			);
			client.provide();

			let clock = RwSignal::new(OPENING);
			let ticking: Clock = clock.into();
			let days = use_cycle_list((1..=DAYS).collect::<Vec<_>>());
			let day = days.state;
			let search = RwSignal::new(String::new());
			let departures = use_query_with(
				sailings(),
				move || day.get().unwrap_or(1),
				// Paging between days keeps the last day's board until the
				// next one lands, instead of flashing empty.
				QueryObserverOptions::new().keep_previous_data(),
			);

			// A departures board reads the day again on its own, for as long
			// as the owner lives.
			use_interval_fn(
				ticking,
				move || {
					departures.refetch();
				},
				REFRESH,
			);

			Self {
				searched: signal_debounced(search, ticking, SEARCH_DELAY).into(),
				departed: use_toggle(false),
				pinned: use_storage(storage, "pinned-route", None)
					.unwrap_or_else(|never| match never {}),
				log: use_infinite_query(log(), || ()),
				book: use_mutation(booking()),
				owner: owner.clone(),
				client,
				clock,
				days,
				search,
				departures,
			}
		})
	}

	/// Moves the harbour clock on, as the host's timer does.
	pub fn advance(&self, by: Duration) {
		self.owner.with(|| self.clock.update(|now| *now += by));
	}

	/// The day the board shows.
	pub fn day(&self) -> u16 {
		self.owner
			.with(|| self.days.state.get_untracked().unwrap_or(1))
	}

	/// The query client, for reading what the cache holds.
	pub fn client(&self) -> &QueryClient {
		&self.client
	}

	/// Whether nothing is fetching or booking: the board shows the harbour
	/// as it is.
	pub fn is_settled(&self) -> bool {
		self.client.fetching_count() == 0 && self.client.mutating_count() == 0
	}

	/// The board for the current state.
	pub fn render(&self) -> Element {
		self.owner.with(|| screen(self))
	}
}

fn screen(board: &Board) -> Element {
	View()
		.class("gap-4 p-6 w-full h-full bg-slate-50")
		.child(header(board))
		.child(controls(board))
		.child(departures(board))
		.child(harbour_log(board))
}

/// The title, the harbour clock, and whether anything is loading.
fn header(board: &Board) -> Element {
	let seconds = board.clock.get().as_secs();
	let clock = format!(
		"{:02}:{:02}:{:02}",
		seconds / 3600 % 24,
		seconds / 60 % 60,
		seconds % 60
	);
	let mut header = View().class("flex-row items-center gap-4").child(
		Heading(1, "Harbour departures").class("flex-1 text-[28px] font-bold text-slate-900"),
	);
	// Announced while anything loads, and gone when nothing does.
	if use_is_fetching().get() > 0 {
		header = header.child(Status("Updating…").class("text-sm text-slate-500"));
	}
	header.child(Text(clock).class("text-[22px] font-semibold text-sky-800"))
}

/// The day picker, the route search, and the departed toggle.
fn controls(board: &Board) -> Element {
	let day = board.days.state.get().unwrap_or(1);
	let previous = board.days.prev.clone();
	let next = board.days.next.clone();
	let search = board.search;
	let showing = board.departed.value.get();
	let toggle = board.departed.toggle.clone();

	View()
		.class("flex-row items-center gap-3")
		.child(Button("Previous day").on_press(move || previous()))
		.child(Text(format!("Day {day}")).class("font-semibold"))
		.child(Button("Next day").on_press(move || next()))
		.child(
			TextInput()
				.aria_label("Search routes")
				.placeholder("Search routes")
				.value(search.get_untracked())
				.on_change(move |text| search.set(text.to_owned()))
				.class("flex-1 h-8 rounded-md px-2 bg-white"),
		)
		.child(
			Button(if showing {
				"Hide departed"
			} else {
				"Show departed"
			})
			.aria_pressed(showing)
			.on_press(move || toggle()),
		)
}

/// The day's sailings: the pinned route first, filtered by the settled
/// search, each with a book and a pin button.
fn departures(board: &Board) -> Element {
	let section = View().class("gap-2 rounded-xl bg-white p-4");
	if board.departures.is_loading() {
		return section.child(Status("Loading sailings…"));
	}
	if let Some(error) = board.departures.error() {
		return section.child(Alert(format!(
			"The board is unavailable: {}",
			error.message()
		)));
	}

	let query = board.searched.get().to_lowercase();
	let pinned = board.pinned.value.get();
	let mut sailings: Vec<Sailing> = board
		.departures
		.data()
		.unwrap_or_default()
		.into_iter()
		.filter(|sailing| board.departed.value.get() || sailing.status != SailingStatus::Departed)
		.filter(|sailing| sailing.route.to_lowercase().contains(&query))
		.collect();
	// A stable sort: the pinned route rises, the rest keep their times.
	sailings.sort_by_key(|sailing| pinned.as_deref() != Some(sailing.route.as_str()));

	let mut table = View()
		.role(Role::Table)
		.aria_label("Departures")
		.class("gap-1");
	for sailing in &sailings {
		table = table.child(departure(
			board,
			sailing,
			pinned.as_deref() == Some(&sailing.route),
		));
	}

	let mut section = section.child(table);
	if sailings.is_empty() {
		section = section.child(Text("No sailings match.").class("text-slate-500"));
	}
	if let Some(error) = board.book.error() {
		section = section.child(Alert(format!("Booking failed: {}", error.message())));
	}
	section
}

/// One sailing's row.
fn departure(board: &Board, sailing: &Sailing, pinned: bool) -> Element {
	let status = match sailing.status {
		SailingStatus::OnTime => "On time".to_owned(),
		SailingStatus::Delayed { minutes } => format!("Delayed {minutes} min"),
		SailingStatus::Departed => "Departed".to_owned(),
	};
	let seats = match sailing.seats_left {
		0 => "Full".to_owned(),
		1 => "1 seat".to_owned(),
		seats => format!("{seats} seats"),
	};
	let bookable = sailing.seats_left > 0
		&& sailing.status != SailingStatus::Departed
		&& !board.book.is_pending();

	let book = {
		let handle = board.book;
		let booking = Booking {
			day: board.days.state.get_untracked().unwrap_or(1),
			sailing: sailing.id,
		};
		move || handle.mutate(booking)
	};
	let pin = {
		let set = board.pinned.set_value.clone();
		let route = sailing.route.clone();
		move || {
			set(if pinned { None } else { Some(route.clone()) })
				.unwrap_or_else(|never| match never {});
		}
	};

	View()
		.role(Role::Row)
		.class("flex-row items-center gap-3")
		.child(
			Text(&sailing.departs)
				.role(Role::Cell)
				.class("w-[56px] font-semibold"),
		)
		.child(Text(&sailing.route).role(Role::Cell).class("flex-1"))
		.child(
			Text(status)
				.role(Role::Cell)
				.class("w-[120px] text-slate-600"),
		)
		.child(Text(seats).role(Role::Cell).class("w-[72px]"))
		.child(
			Button("Book")
				.aria_label(format!("Book {}", sailing.route))
				.enabled(bookable)
				.on_press(book),
		)
		.child(
			Button(if pinned { "★" } else { "☆" })
				.aria_label(format!("Pin {}", sailing.route))
				.aria_pressed(pinned)
				.on_press(pin),
		)
}

/// The harbour master's log, with older pages on request.
fn harbour_log(board: &Board) -> Element {
	let log = board.log;
	let entries = log.items_with(|page| &page.entries);
	let mut section = View()
		.class("gap-1 rounded-xl bg-white p-4")
		.child(Heading(2, "Harbour log").class("font-semibold"));
	for entry in entries {
		section = section.child(Text(entry.text).class("text-sm text-slate-700"));
	}
	section.child(
		Button("Earlier entries")
			.enabled(log.has_next_page() && !log.is_fetching_next_page())
			.on_press(move || {
				log.fetch_next_page();
			}),
	)
}
