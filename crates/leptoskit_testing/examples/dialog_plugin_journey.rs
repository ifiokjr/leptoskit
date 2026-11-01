//! Real-OS journey for `leptoskit_plugin_dialog`'s `native_fixture` window on
//! Windows, macOS, and Linux: presents the plugin's file chooser and message
//! box, answers them through the operating system's own controls, and asserts
//! the returned result.

#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	journey::run()
}

#[cfg(not(all(feature = "native-ui", not(target_arch = "wasm32"))))]
fn main() {
	eprintln!("dialog_plugin_journey requires --features native-ui on a desktop host");
	std::process::exit(1);
}

#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
mod journey {
	use leptoskit_testing::native::Application;
	use leptoskit_testing::native::Error;
	use leptoskit_testing::native::Locator;
	use leptoskit_testing::native::NativeTester;
	use leptoskit_testing::native::Options;
	use leptoskit_testing::native::Platform;
	use serde_json::json;

	/// Drives the journey the arguments ask for.
	pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
		let args = Args::parse(std::env::args().skip(1).collect())?;
		let options = args.options();

		NativeTester::run(options, |tester| {
			tester.expect_visible(&Locator::text("Ready"))?;
			tester.checkpoint("Fixture window before any dialog")?;

			file_chooser(tester, args.platform, args.accept)?;
			message_box(tester, args.platform, args.accept)?;
			check_capture(tester)?;

			println!(
				"Native report: {}",
				tester.artifacts().join("index.html").display()
			);

			Ok(())
		})?;

		Ok(())
	}

	/// What the journey was asked to drive.
	struct Args {
		platform: Platform,
		endpoint: String,
		application: Application,
		accept: bool,
	}

	impl Args {
		/// Reads `<windows|macos|linux> <endpoint> <app|Root>
		/// <accept|decline>`.
		fn parse(args: Vec<String>) -> Result<Self, String> {
			if args.len() != 4 {
				return Err(
					"usage: dialog_plugin_journey <windows|macos|linux> <endpoint> <app|Root> \
					 <accept|decline>"
						.into(),
				);
			}

			let platform = match args[0].as_str() {
				"windows" => Platform::Windows,
				"macos" => Platform::Macos,
				"linux" => Platform::Linux,
				_ => return Err("expected windows, macos or linux".into()),
			};
			let accept = match args[3].as_str() {
				"accept" => true,
				"decline" => false,
				_ => return Err("expected accept or decline".into()),
			};
			let application = if args[2] == "Root" {
				Application::Desktop
			} else {
				Application::Path(args[2].clone())
			};

			Ok(Self {
				platform,
				endpoint: args[1].clone(),
				application,
				accept,
			})
		}

		/// The session options, with the longer budgets macOS's first run
		/// needs for its XCTest runner.
		fn options(&self) -> Options {
			let verdict = if self.accept { "accept" } else { "decline" };
			let platform = match self.platform {
				Platform::Windows => "windows",
				Platform::Macos => "macos",
				Platform::Linux => "linux",
				Platform::Ios => "ios",
				Platform::Android => "android",
			};
			let mut options = Options::new(&self.endpoint, self.platform, self.application.clone());
			options.artifacts =
				format!("target/native-ui/reports/dialog-plugin-{platform}-{verdict}").into();

			if self.platform == Platform::Macos {
				// The first session builds the driver's XCTest runner with
				// Xcode.
				options.startup_seconds = 900;
				options.wait_seconds = 30;
				// Keep the runner's Xcode output in the driver log for
				// diagnosis.
				options
					.capabilities
					.insert("appium:showServerLogs".to_owned(), json!(true));
			}

			options
		}
	}

	/// Presents the plugin's file chooser and answers it.
	fn file_chooser(
		tester: &mut NativeTester,
		platform: Platform,
		accept: bool,
	) -> Result<(), Error> {
		if platform == Platform::Linux {
			tester.input_key(' ')?;
		} else {
			tester.tap(&Locator::text("Choose evidence"))?;
		}
		let choice = dialog_button(
			platform,
			"Choose evidence",
			if accept { "Open" } else { "Cancel" },
		);
		tester.expect_visible(&choice)?;
		tester.checkpoint("The plugin's native file chooser")?;

		if platform == Platform::Linux {
			answer_with_keys(tester, accept, "evidence.txt")?;
		} else {
			tester.tap(&choice)?;
		}
		tester.expect_visible(&Locator::text(if accept {
			"Selected: evidence.txt"
		} else {
			"Cancelled"
		}))?;

		tester.expect_absent(&choice)?;
		tester.checkpoint("File chooser result returned to the app")
	}

	/// Presents the plugin's message box and answers it.
	fn message_box(
		tester: &mut NativeTester,
		platform: Platform,
		accept: bool,
	) -> Result<(), Error> {
		if platform == Platform::Linux {
			// Focus returns to the fixture's first button when the chooser
			// closes; Tab moves to Confirm, and Space presents the box.
			tester.input_key('\t')?;
			tester.input_key(' ')?;
		} else {
			tester.tap(&Locator::text("Confirm"))?;
		}
		let answer = if platform == Platform::Linux {
			// zenity's GTK buttons each mirror their label into a child
			// accessible, so the message's own text is the unique proof of the
			// box; its default answer is Ok.
			Locator::text("Keep the selected evidence?")
		} else {
			dialog_button(
				platform,
				"Confirm evidence",
				if accept { "OK" } else { "Cancel" },
			)
		};

		tester.expect_visible(&answer)?;
		tester.checkpoint("The plugin's native message box")?;

		if platform == Platform::Linux {
			tester.input_key(if accept { '\r' } else { '\u{1b}' })?;
		} else {
			tester.tap(&answer)?;
		}
		tester.expect_visible(&Locator::text(if accept {
			"Confirmed"
		} else {
			"Declined"
		}))?;

		tester.expect_absent(&answer)?;
		tester.checkpoint("Message box result returned to the app")
	}

	/// Answers a GTK dialog with keys, since no control can be tapped.
	///
	/// GTK's AT-SPI bridge reports every component at the origin and the
	/// driver never performs GTK's `click` action. The fixture starts with its
	/// first button focused, and zenity answers Enter and Escape; the chooser
	/// opens on the suggested folder with nothing preselected, so accepting
	/// types the suggested file's name.
	fn answer_with_keys(tester: &mut NativeTester, accept: bool, name: &str) -> Result<(), Error> {
		if accept {
			tester.input_type(name)?;
			tester.input_key('\r')?;
		} else {
			tester.input_key('\u{1b}')?;
		}

		Ok(())
	}

	/// Addresses a dialog button the way each platform's driver exposes it.
	fn dialog_button(platform: Platform, dialog: &str, label: &str) -> Locator {
		if platform == Platform::Linux {
			// The AT-SPI driver's XPath strategy only walks Qt accessibles,
			// which hides the GTK fixture and zenity's message box from it.
			// The name strategy searches the whole desktop, where KDE's portal
			// file chooser and zenity's buttons are uniquely named.
			return Locator::text(label);
		}

		if platform == Platform::Macos {
			// A macOS session sees only the fixture, and both dialogs are
			// sheets on its window.
			return Locator::Xpath(format!(
				"//XCUIElementTypeSheet//XCUIElementTypeButton[@title='{label}']"
			));
		}

		// Hosted runners can show unrelated system dialogs whose buttons share
		// these names (a paging-file warning has its own "OK"), so address each
		// button through the fixture's own owned dialog.
		let owner = "//Window[@ClassName='LeptoskitDialogFixture']";
		let button = format!("//Window[@Name='{dialog}']//Button[@Name='{label}']");

		Locator::Xpath(format!("{owner}{button}"))
	}

	/// Fails when the capture is a flat placeholder rather than a window.
	fn check_capture(tester: &mut NativeTester) -> Result<(), Error> {
		let capture = tester.screenshot()?;

		if capture.distinct_colors(32)? <= 32 {
			return Err(Error::Assertion(
				"capture has too few colors for a rendered native window".to_owned(),
			));
		}

		Ok(())
	}
}
