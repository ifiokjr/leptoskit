//! The binary itself: a scripted morning at the harbour runs to the end, and
//! `leptoskit rpc` can ask it for its spec.

use std::process::Command;

use leptoskit::rpc::spec::SPEC_OUT_ENV;

fn harbor() -> Command {
	Command::new(env!("CARGO_BIN_EXE_harbor_example"))
}

#[test]
fn a_morning_at_the_harbour_runs_to_the_end() {
	let output = harbor().output().expect("the binary runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let story = String::from_utf8_lossy(&output.stdout);
	for step in [
		"── The board opens on day 1",
		"── A seat on the 08:15 to Lighthouse Point is booked",
		"── The search, once typing pauses, finds Seal Rocks",
		"── An older page of the harbour log is loaded",
	] {
		assert!(story.contains(step), "{story}");
	}
	assert!(story.contains("text \"Kelp Island pier repainted.\""));
}

#[test]
fn asked_for_its_spec_the_app_writes_it_and_stops() {
	let path = std::env::temp_dir().join(format!("harbour-spec-{}.json", std::process::id()));
	let output = harbor()
		.env(SPEC_OUT_ENV, &path)
		.output()
		.expect("the binary runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(
		String::from_utf8_lossy(&output.stdout).is_empty(),
		"the board never ran"
	);
	let spec = std::fs::read_to_string(&path).expect("the spec was written");
	std::fs::remove_file(&path).ok();
	for function in ["sailings", "book_seat", "harbor_log"] {
		assert!(spec.contains(&format!("\"{function}\"")), "{spec}");
	}
}
