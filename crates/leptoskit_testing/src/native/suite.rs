//! Serializable native scenarios for local runs and platform CI.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::Error;
use super::Locator;
use super::NativeTester;
use super::Options;

/// An ordered scenario. Every assertion and action uses the real native driver.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
	/// Session configuration.
	pub session: Options,
	/// Steps executed in order, stopping on the first failure.
	pub steps: Vec<Step>,
}

/// Native scenario operations. Screenshot steps embed real PNGs in the report.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
	/// Click a visible enabled control.
	Tap { locator: Locator },
	/// Replace input text.
	EnterText { locator: Locator, text: String },
	/// Wait for exactly one visible control.
	ExpectVisible { locator: Locator },
	/// Wait for every matching control to disappear.
	ExpectAbsent { locator: Locator },
	/// Wait for an exact visible text value.
	ExpectText { locator: Locator, text: String },
	/// Accept a native alert.
	AcceptAlert,
	/// Dismiss a native alert.
	DismissAlert,
	/// Save actual screen pixels and the accessibility tree.
	Screenshot { label: String },
	/// Execute a documented platform driver extension.
	NativeCommand { script: String, arguments: Value },
	/// Dispatch W3C pointer/key action sources.
	PerformActions { sources: Value },
}

impl Suite {
	/// Runs the scenario, retaining evidence and closing the session on
	/// failure.
	pub fn run(self) -> Result<std::path::PathBuf, Error> {
		if self.steps.is_empty() {
			return Err(Error::Configuration("native suite has no steps".to_owned()));
		}
		NativeTester::run(self.session, |tester| {
			for step in self.steps {
				match step {
					Step::Tap { locator } => tester.tap(&locator)?,
					Step::EnterText { locator, text } => tester.enter_text(&locator, &text)?,
					Step::ExpectVisible { locator } => tester.expect_visible(&locator)?,
					Step::ExpectAbsent { locator } => tester.expect_absent(&locator)?,
					Step::ExpectText { locator, text } => tester.expect_text(&locator, &text)?,
					Step::AcceptAlert => tester.accept_alert()?,
					Step::DismissAlert => tester.dismiss_alert()?,
					Step::Screenshot { label } => tester.checkpoint(&label)?,
					Step::NativeCommand { script, arguments } => {
						tester.native_command(&script, &arguments)?;
					}
					Step::PerformActions { sources } => tester.perform_actions(&sources)?,
				}
			}
			Ok(tester.artifacts().to_path_buf())
		})
	}
}
