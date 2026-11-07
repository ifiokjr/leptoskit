//! Reading cargo's `--message-format=json` streams: the artifacts the CLI
//! collects, with the compiler's own diagnostics printed instead of dropped.
//!
//! `json-render-diagnostics` puts every warning and error inside the JSON
//! stream, not on the terminal, so a reader that only picks artifacts
//! silently swallows them — including the errors that explain a failure.

use std::io::BufRead;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

/// One line of cargo's JSON message stream. Fields the CLI doesn't read are
/// absent from the type and ignored by serde.
#[derive(Debug, Deserialize)]
pub struct Message {
	pub reason: String,
	/// The fully rendered diagnostic of a `compiler-message`, trailing
	/// newline included.
	#[serde(default)]
	pub rendered: String,
	#[serde(default)]
	pub executable: Option<PathBuf>,
	#[serde(default)]
	pub filenames: Vec<PathBuf>,
	/// The `Cargo.toml` of the package a `compiler-artifact` belongs to,
	/// which tells the requested package's artifacts from its dependencies'.
	#[serde(default)]
	pub manifest_path: Option<PathBuf>,
	#[serde(default)]
	pub target: Option<Target>,
}

#[derive(Debug, Deserialize)]
pub struct Target {
	#[serde(default)]
	pub kind: Vec<String>,
}

/// Streams cargo's messages, printing each rendered diagnostic to stderr,
/// and hands every parsed message to `sink` for artifact collection. Lines
/// that are not messages (or not valid JSON) are skipped, as cargo may print
/// its own notes around the stream.
pub fn stream<F>(messages: impl BufRead, mut sink: F) -> Result<()>
where
	F: FnMut(&Message),
{
	for line in messages.lines() {
		let line = line.context("could not read cargo's output")?;
		let Ok(message) = serde_json::from_str::<Message>(&line) else {
			continue;
		};

		if let Some(diagnostic) = rendered(&message) {
			eprint!("{diagnostic}");
		}

		sink(&message);
	}

	Ok(())
}

/// The diagnostic a message carries for the terminal, rendered by rustc with
/// its trailing newline.
fn rendered(message: &Message) -> Option<&str> {
	(message.reason == "compiler-message" && !message.rendered.is_empty())
		.then_some(message.rendered.as_str())
}

#[cfg(test)]
mod tests {
	use super::Message;
	use super::rendered;
	use super::stream;

	fn message(reason: &str, rendered: &str) -> Message {
		serde_json::from_value(serde_json::json!({
			"reason": reason,
			"rendered": rendered,
		}))
		.unwrap()
	}

	#[test]
	fn only_rendered_compiler_messages_carry_a_diagnostic() {
		assert_eq!(
			rendered(&message(
				"compiler-message",
				"warning: unused variable `x`\n"
			)),
			Some("warning: unused variable `x`\n")
		);
		assert_eq!(rendered(&message("compiler-message", "")), None);
		assert_eq!(
			rendered(&message("compiler-artifact", "irrelevant\n")),
			None
		);
	}

	#[test]
	fn mixed_output_reaches_the_sink_without_failing() {
		let messages = concat!(
			"note: some unrelated cargo output\n",
			"{\"reason\":\"compiler-message\",\"rendered\":\"warning: unused variable\\n\"}\n",
			"{\"reason\":\"compiler-artifact\",\"executable\":\"/bin/app\",\"filenames\":[]}\n",
			"{\"reason\":\"build-finished\",\"success\":true}\n"
		);
		let mut reasons = Vec::new();
		let mut artifacts = Vec::new();
		stream(messages.as_bytes(), |message| {
			reasons.push(message.reason.clone());

			if message.reason == "compiler-artifact" {
				artifacts.push(message.executable.clone());
			}
		})
		.unwrap();
		assert_eq!(
			reasons,
			["compiler-message", "compiler-artifact", "build-finished"]
		);
		assert_eq!(artifacts, [Some("/bin/app".into())]);
	}
}
