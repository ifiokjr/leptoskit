//! The scripted session: an agent that finds and fixes a timer bug, with no
//! model and no network behind it.

use crate::Agent;
use crate::Change;
use crate::DiffLine;
use crate::Message;
use crate::Part;
use crate::Session;

const FILE: &str = "pub struct Timer {
    seconds: u64,
}

impl Timer {
    pub fn new(seconds: u64) -> Self {
        Self { seconds }
    }

    pub fn label(&self) -> String {
        let minutes = self.seconds / 60;
        let seconds = self.seconds / 60;
        format!(\"{minutes}:{seconds:02}\")
    }
}
";

/// The session a first prompt starts: the agent reads the timer, fixes
/// its seconds, runs the tests, and explains the fix.
#[must_use]
pub fn session(prompt: &str, agent: Agent) -> Session {
	Session {
		title: "Fix timer seconds formatting".to_owned(),
		messages: vec![
			Message::User(prompt.to_owned()),
			Message::Assistant {
				parts: vec![
					Part::Text("I'll find the timer and read how it formats the label.".to_owned()),
					Part::Explored("Explored — 1 search, 1 read".to_owned()),
					Part::Text(
						"The seconds use `/` instead of `%`, so 90 seconds prints `1:01`. Fixing \
						 it:"
						.to_owned(),
					),
					Part::Edit {
						path: "src/timer.rs".to_owned(),
						lines: diff(),
					},
					Part::Shell {
						command: "cargo test --quiet".to_owned(),
						output: "running 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 \
						         ignored; 0 measured; 0 filtered out; finished in 0.00s"
							.to_owned(),
					},
					Part::Text(
						"Fixed. `Timer::label` divided the seconds by 60 a second time instead of \
						 taking the remainder.\n\n- **src/timer.rs**: `self.seconds / 60` → \
						 `self.seconds % 60`\n- `cargo test` passes: \
						 `formats_minutes_and_seconds` now sees `1:30`.\n\n```rust\nlet minutes = \
						 self.seconds / 60;\nlet seconds = self.seconds % 60;\n```"
							.to_owned(),
					),
				],
				agent,
				timing: "2.4s · 3295.2 tok/s".to_owned(),
			},
		],
		tokens: 19_254,
	}
}

/// The reply to every later prompt: this demo has no model to ask.
#[must_use]
pub fn offline_reply(agent: Agent) -> Message {
	Message::Assistant {
		parts: vec![Part::Text(
			"This demo runs offline, so every reply is scripted. The interface is real: the same \
			 `Element` tree renders in your terminal and in the browser."
				.to_owned(),
		)],
		agent,
		timing: "0.1s · 812.0 tok/s".to_owned(),
	}
}

/// Lines 8–16 of the timer around the fix, as the edit tool shows them.
fn diff() -> Vec<DiffLine> {
	let lines: Vec<&str> = FILE.lines().collect();
	let kept = |number: u32| {
		DiffLine {
			number,
			change: Change::Kept,
			code: lines
				.get(number as usize - 1)
				.copied()
				.unwrap_or_default()
				.to_owned(),
		}
	};

	let mut diff: Vec<DiffLine> = (8..=11).map(kept).collect();
	diff.push(DiffLine {
		number: 12,
		change: Change::Removed,
		code: lines[11].to_owned(),
	});
	diff.push(DiffLine {
		number: 12,
		change: Change::Added,
		code: lines[11].replace("/ 60", "% 60"),
	});
	diff.extend((13..=16).map(kept));
	diff
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_diff_changes_only_the_seconds_line() {
		let diff = diff();
		let changed: Vec<_> = diff
			.iter()
			.filter(|line| line.change != Change::Kept)
			.map(|line| line.code.trim())
			.collect();
		assert_eq!(
			changed,
			vec![
				"let seconds = self.seconds / 60;",
				"let seconds = self.seconds % 60;"
			]
		);
		assert_eq!(diff.first().map(|line| line.number), Some(8));
		assert_eq!(diff.last().map(|line| line.number), Some(16));
	}
}
