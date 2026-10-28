//! Runs the board through a short morning at the harbour and prints what a
//! screen reader would read at each step.
//!
//! ```sh
//! cargo run -p harbor_example
//! ```
//!
//! Run by `leptoskit rpc`, it writes the RPC's spec instead, which is how the
//! clients in `clients/` are generated.

use std::time::Duration;

use harbor_example::board::Board;
use harbor_example::board::VIEWPORT;
use leptoskit::hooks::MemoryStorage;
use leptoskit::native::Role;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;

#[tokio::main]
async fn main() -> Result<(), TestError> {
	// Writes the spec and exits when `leptoskit rpc` asks for it, and refuses
	// to start when two functions share a route.
	leptoskit::rpc::spec::export_if_requested();
	any_spawner::Executor::init_tokio().ok();
	let board = Board::new(MemoryStorage::default());
	let render = board.clone();
	let mut tester = WidgetTester::new(move || render.render(), VIEWPORT.0, VIEWPORT.1)?;

	step(&board, &mut tester, "The board opens on day 1").await;

	tester.tap(&Finder::label("Book Lighthouse Point"))?;
	step(
		&board,
		&mut tester,
		"A seat on the 08:15 to Lighthouse Point is booked",
	)
	.await;

	tester.enter_text(&Finder::label("Search routes"), "rock")?;
	board.advance(Duration::from_millis(400));
	step(
		&board,
		&mut tester,
		"The search, once typing pauses, finds Seal Rocks",
	)
	.await;

	tester.tap(&Finder::aria_role(Role::Button, "Earlier entries"))?;
	step(
		&board,
		&mut tester,
		"An older page of the harbour log is loaded",
	)
	.await;
	Ok(())
}

/// Lets the queries and bookings land, renders, and prints the screen.
async fn step(board: &Board, tester: &mut WidgetTester, title: &str) {
	loop {
		any_spawner::Executor::tick().await;
		tokio::time::sleep(Duration::from_millis(5)).await;
		if board.is_settled() {
			break;
		}
	}
	tester.pump();
	println!("── {title}\n{}", tester.semantics());
}
