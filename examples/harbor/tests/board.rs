//! The board the way a passenger uses it, over the real renderer on the
//! headless backend: queries landing, a booking that shows before the server
//! answers and rolls back when it refuses, days, search, the clock, pins, and
//! the log.
//!
//! The harbour is one in-memory server shared by every board in a process,
//! like a real backend; each test books its own sailings, so none depends on
//! another's bookings.

use std::time::Duration;

use harbor_example::board::Board;
use harbor_example::board::REFRESH;
use harbor_example::board::SEARCH_DELAY;
use harbor_example::board::VIEWPORT;
use harbor_example::board::sailings;
use leptoskit::hooks::MemoryStorage;
use leptoskit::native::Role;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;

async fn mount(storage: MemoryStorage<Option<String>>) -> Result<(Board, WidgetTester), TestError> {
	any_spawner::Executor::init_tokio().ok();
	let board = Board::new(storage);
	let render = board.clone();
	let mut tester = WidgetTester::new(move || render.render(), VIEWPORT.0, VIEWPORT.1)?;
	settle(&board, &mut tester).await;
	Ok((board, tester))
}

/// Lets the executor run spawned work for a moment, then renders.
async fn tick(tester: &mut WidgetTester) {
	any_spawner::Executor::tick().await;
	tokio::time::sleep(Duration::from_millis(5)).await;
	tester.pump();
}

/// Runs until every fetch and booking has landed, then renders. The harbour
/// takes a while to answer, so this waits on the board rather than a guess.
async fn settle(board: &Board, tester: &mut WidgetTester) {
	tick(tester).await;
	for _ in 0..400 {
		if board.is_settled() {
			tick(tester).await;
			return;
		}
		tick(tester).await;
	}
	panic!("the board did not settle");
}

/// The seats cell of a route's row, as it reads.
fn seats(tester: &WidgetTester, route: &str) -> String {
	committed_cells(tester)
		.into_iter()
		.skip_while(|cell| cell != route)
		.nth(2)
		.unwrap_or_else(|| panic!("{route} has a seats cell"))
}

/// Every committed cell's text, in tree order.
fn committed_cells(tester: &WidgetTester) -> Vec<String> {
	tester
		.semantics()
		.lines()
		.filter_map(|line| line.trim().strip_prefix("cell \""))
		.map(|rest| rest.trim_end_matches('"').to_owned())
		.collect()
}

fn routes(tester: &WidgetTester) -> Vec<String> {
	committed_cells(tester)
		.chunks(4)
		.map(|row| row[1].clone())
		.collect()
}

#[tokio::test]
async fn the_board_opens_on_the_first_day_without_departed_sailings() -> Result<(), TestError> {
	let (board, tester) = mount(MemoryStorage::default()).await?;
	assert_eq!(board.day(), 1);
	tester.find(&Finder::text("Day 1"))?;
	assert_eq!(
		routes(&tester),
		["Lighthouse Point", "The Old Quay", "Seal Rocks", "Mainland"],
		"the 06:40 to Kelp Island has left"
	);
	assert!(board.client().get_query_data(&sailings(), &1).is_some());
	Ok(())
}

#[tokio::test]
async fn departed_sailings_show_on_request_and_cannot_be_booked() -> Result<(), TestError> {
	let (_, mut tester) = mount(MemoryStorage::default()).await?;
	tester.tap(&Finder::aria_role(Role::Button, "Show departed"))?;
	assert_eq!(routes(&tester)[0], "Kelp Island");
	tester.find(&Finder::aria_role(Role::Cell, "Departed"))?;
	assert_eq!(
		tester
			.find(&Finder::label("Book Kelp Island"))?
			.props
			.enabled,
		Some(false)
	);
	Ok(())
}

#[tokio::test]
async fn a_booking_takes_a_seat_at_once_and_keeps_it() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	// Day 4 is this test's own.
	for _ in 0..3 {
		tester.tap(&Finder::aria_role(Role::Button, "Next day"))?;
	}
	settle(&board, &mut tester).await;
	tester.find(&Finder::text("Day 4"))?;
	let before: u32 = seat_count(&seats(&tester, "Mainland"));

	tester.tap(&Finder::label("Book Mainland"))?;
	// The optimistic update shows well before the server can answer.
	tick(&mut tester).await;
	assert!(!board.is_settled(), "the booking is still on its way");
	assert_eq!(seat_count(&seats(&tester, "Mainland")), before - 1);
	settle(&board, &mut tester).await;
	assert_eq!(seat_count(&seats(&tester, "Mainland")), before - 1);
	assert!(tester.find(&Finder::aria_role(Role::Alert, "")).is_err());
	Ok(())
}

#[tokio::test]
async fn a_booking_the_server_refuses_rolls_back_and_says_why() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	// Day 2's Kelp Island sailing has one seat, and this test has it to
	// itself.
	tester.tap(&Finder::aria_role(Role::Button, "Next day"))?;
	settle(&board, &mut tester).await;
	assert_eq!(seats(&tester, "Kelp Island"), "1 seat");

	// Another passenger takes it while the board's copy is still fresh.
	harbor_example::api::book_seat(200)
		.await
		.expect("the last seat is free");
	tester.pump();
	assert_eq!(
		seats(&tester, "Kelp Island"),
		"1 seat",
		"the board has not looked again"
	);

	// The board takes the seat at once, the server refuses, and the board
	// rolls back, says why, and reads the day again: now it is full.
	tester.tap(&Finder::label("Book Kelp Island"))?;
	tick(&mut tester).await;
	assert_eq!(seats(&tester, "Kelp Island"), "Full");
	settle(&board, &mut tester).await;
	tester.find(&Finder::aria_role(Role::Alert, "Booking failed: sold out"))?;
	assert_eq!(seats(&tester, "Kelp Island"), "Full");
	assert_eq!(
		tester
			.find(&Finder::label("Book Kelp Island"))?
			.props
			.enabled,
		Some(false)
	);
	Ok(())
}

#[tokio::test]
async fn the_search_waits_for_typing_to_pause() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	tester.enter_text(&Finder::label("Search routes"), "quay")?;
	assert_eq!(routes(&tester).len(), 4, "still typing");

	board.advance(SEARCH_DELAY / 2);
	tester.pump();
	assert_eq!(routes(&tester).len(), 4, "still waiting");

	board.advance(SEARCH_DELAY);
	tester.pump();
	assert_eq!(routes(&tester), ["The Old Quay"]);

	tester.enter_text(&Finder::label("Search routes"), "zzz")?;
	board.advance(SEARCH_DELAY);
	tester.pump();
	tester.find(&Finder::text("No sailings match."))?;
	Ok(())
}

#[tokio::test]
async fn the_harbour_clock_ticks_with_the_host() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	tester.find(&Finder::text("06:00:00"))?;
	board.advance(Duration::from_secs(61));
	tester.pump();
	tester.find(&Finder::text("06:01:01"))?;
	Ok(())
}

#[tokio::test]
async fn the_board_reads_the_day_again_every_half_minute() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	let before = seat_count(&seats(&tester, "The Old Quay"));

	// Another passenger books; the board's copy is still fresh.
	harbor_example::api::book_seat(102)
		.await
		.expect("seats are free");
	board.advance(REFRESH / 2);
	settle(&board, &mut tester).await;
	assert_eq!(seat_count(&seats(&tester, "The Old Quay")), before);

	// Half a minute on, the board has looked again.
	board.advance(REFRESH / 2);
	settle(&board, &mut tester).await;
	assert_eq!(seat_count(&seats(&tester, "The Old Quay")), before - 1);
	Ok(())
}

#[tokio::test]
async fn a_pinned_route_rises_to_the_top_and_stays_pinned() -> Result<(), TestError> {
	let storage = MemoryStorage::default();
	let (_, mut tester) = mount(storage.clone()).await?;
	tester.tap(&Finder::label("Pin Mainland"))?;
	assert_eq!(routes(&tester)[0], "Mainland");
	assert_eq!(
		tester
			.find(&Finder::label("Pin Mainland"))?
			.props
			.aria()
			.pressed,
		Some(true.into())
	);

	// A board started over the same storage remembers the pin.
	let (_, tester) = mount(storage).await?;
	assert_eq!(routes(&tester)[0], "Mainland");
	Ok(())
}

#[tokio::test]
async fn the_log_pages_back_to_its_first_entry() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	let entries = |tester: &WidgetTester| {
		tester
			.semantics()
			.lines()
			.skip_while(|line| !line.contains("Harbour log"))
			.filter(|line| line.starts_with("text "))
			.count()
	};
	assert_eq!(entries(&tester), 4);
	tester.find(&Finder::text("Storm shutters down until Thursday."))?;

	tester.tap(&Finder::aria_role(Role::Button, "Earlier entries"))?;
	settle(&board, &mut tester).await;
	assert_eq!(entries(&tester), 8);

	tester.tap(&Finder::aria_role(Role::Button, "Earlier entries"))?;
	settle(&board, &mut tester).await;
	assert_eq!(entries(&tester), 10);
	tester.find(&Finder::text("Harbour opens for the season."))?;
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "Earlier entries"))?
			.props
			.enabled,
		Some(false)
	);
	Ok(())
}

#[tokio::test]
async fn paging_days_keeps_the_last_board_until_the_next_lands() -> Result<(), TestError> {
	let (board, mut tester) = mount(MemoryStorage::default()).await?;
	tester.tap(&Finder::aria_role(Role::Button, "Previous day"))?;
	assert_eq!(board.day(), 7, "the week wraps");
	// Before the fetch lands, the previous day's board is still on screen.
	assert!(!routes(&tester).is_empty());
	settle(&board, &mut tester).await;
	tester.find(&Finder::text("Day 7"))?;
	assert!(board.client().get_query_data(&sailings(), &7).is_some());
	Ok(())
}

fn seat_count(cell: &str) -> u32 {
	cell.split(' ')
		.next()
		.and_then(|count| count.parse().ok())
		.unwrap_or_else(|| panic!("{cell:?} counts seats"))
}
