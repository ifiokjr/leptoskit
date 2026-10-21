//! Real-OS journey for `leptoskit_plugin_dialog`'s `native_fixture` window:
//! presents the plugin's file chooser and message box, answers them through
//! the operating system's own controls, and asserts the returned result.
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	use leptoskit_testing::native::Application;
	use leptoskit_testing::native::Error;
	use leptoskit_testing::native::Locator;
	use leptoskit_testing::native::NativeTester;
	use leptoskit_testing::native::Options;
	use leptoskit_testing::native::Platform;
	let args: Vec<String> = std::env::args().skip(1).collect();
	if args.len() != 2 {
		return Err("usage: dialog_plugin_journey <endpoint> <accept|decline>".into());
	}
	let accept = match args[1].as_str() {
		"accept" => true,
		"decline" => false,
		_ => return Err("expected accept or decline".into()),
	};
	let mut options = Options::new(&args[0], Platform::Windows, Application::Desktop);
	options.artifacts = format!("target/native-ui/reports/dialog-plugin-{}", args[1]).into();
	NativeTester::run(options, |tester| {
		tester.expect_visible(&Locator::text("Ready"))?;
		tester.checkpoint("Fixture window before any dialog")?;

		tester.tap(&Locator::text("Choose evidence"))?;
		let choice = Locator::text(if accept { "Open" } else { "Cancel" });
		tester.expect_visible(&choice)?;
		tester.checkpoint("The plugin's native file chooser")?;
		tester.tap(&choice)?;
		tester.expect_visible(&Locator::text(if accept {
			"Selected: evidence.txt"
		} else {
			"Cancelled"
		}))?;
		tester.expect_absent(&choice)?;
		tester.checkpoint("File chooser result returned to the app")?;

		tester.tap(&Locator::text("Confirm"))?;
		let answer = Locator::text(if accept { "OK" } else { "Cancel" });
		tester.expect_visible(&answer)?;
		tester.checkpoint("The plugin's native message box")?;
		tester.tap(&answer)?;
		tester.expect_visible(&Locator::text(if accept {
			"Confirmed"
		} else {
			"Declined"
		}))?;
		tester.expect_absent(&answer)?;
		tester.checkpoint("Message box result returned to the app")?;

		// A flat placeholder image cannot satisfy the capture contract.
		let capture = tester.screenshot()?;
		if capture.distinct_colors(32)? <= 32 {
			return Err(Error::Assertion(
				"capture has too few colors for a rendered native window".to_owned(),
			));
		}
		println!(
			"Native report: {}",
			tester.artifacts().join("index.html").display()
		);
		Ok(())
	})?;
	Ok(())
}

#[cfg(not(all(feature = "native-ui", not(target_arch = "wasm32"))))]
fn main() {
	eprintln!("dialog_plugin_journey requires --features native-ui on a desktop host");
	std::process::exit(1);
}
