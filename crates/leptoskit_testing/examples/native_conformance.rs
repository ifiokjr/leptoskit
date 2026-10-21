//! Real-platform conformance journey for the native fixtures. See the native
//! testing guide.
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	use leptoskit_testing::native::Application;
	use leptoskit_testing::native::Error;
	use leptoskit_testing::native::Locator;
	use leptoskit_testing::native::NativeTester;
	use leptoskit_testing::native::Options;
	use leptoskit_testing::native::Platform;
	use serde_json::json;
	let args: Vec<String> = std::env::args().skip(1).collect();
	if args.len() != 5 {
		return Err(
			"usage: native_conformance <ios|android|windows|linux> <endpoint> <app|Root> \
			 <device|desktop> <allow|deny>"
				.into(),
		);
	}
	let platform = match args[0].as_str() {
		"ios" => Platform::Ios,
		"android" => Platform::Android,
		"windows" => Platform::Windows,
		"linux" => Platform::Linux,
		_ => return Err("unknown platform".into()),
	};
	let allow = match args[4].as_str() {
		"allow" => true,
		"deny" => false,
		_ => return Err("expected allow or deny".into()),
	};
	let application = if args[2] == "Root" {
		Application::Desktop
	} else {
		Application::Path(args[2].clone())
	};
	let mut options = Options::new(&args[1], platform, application);
	options.artifacts = format!("target/native-ui/reports/{}-{}", args[0], args[4]).into();
	if matches!(platform, Platform::Ios | Platform::Android) {
		options.device = Some(args[3].clone());
	}
	if platform == Platform::Android {
		options
			.capabilities
			.insert("appium:appActivity".to_owned(), json!(".CaptureActivity"));
		options
			.capabilities
			.insert("appium:systemPort".to_owned(), json!(8281));
	}
	if platform == Platform::Ios {
		// Cold simulators and WDA builds each need their own bounded window.
		// The session budget also covers app installation and launch.
		options.startup_seconds = 900;
		// Hosted SpringBoard can take over 15 seconds to present permission UI.
		options.wait_seconds = 60;
		options
			.capabilities
			.insert("appium:simulatorStartupTimeout".to_owned(), json!(300_000));
		options
			.capabilities
			.insert("appium:wdaLaunchTimeout".to_owned(), json!(300_000));
		options
			.capabilities
			.insert("appium:showXcodeLog".to_owned(), json!(true));
		options
			.capabilities
			.insert("appium:wdaLocalPort".to_owned(), json!(8181));
		options
			.capabilities
			.insert("appium:isHeadless".to_owned(), json!(true));
	}
	let control = |id: &str| {
		if platform == Platform::Linux {
			Locator::text(id)
		} else {
			Locator::accessibility_id(id)
		}
	};
	NativeTester::run(options, |tester| {
		tester.enter_text(&control("name"), "Leptoskit")?;
		tester.tap(&control("greet"))?;
		tester.expect_visible(&Locator::text("Hello, Leptoskit"))?;
		tester.tap(&control("increment"))?;
		tester.expect_visible(&Locator::text("Count: 1"))?;
		tester.checkpoint("Rendered native text and controls")?;
		tester.tap(&control("permission"))?;
		let choice = match platform {
			Platform::Android => {
				Locator::Id(format!(
					"com.android.permissioncontroller:id/permission_{}_button",
					if allow { "allow" } else { "deny" }
				))
			}
			Platform::Ios => Locator::text(if allow { "Allow" } else { "Don’t Allow" }),
			Platform::Windows | Platform::Linux => {
				Locator::text(if allow { "Open" } else { "Cancel" })
			}
		};
		tester.expect_visible(&choice)?;
		tester.checkpoint("Real operating-system dialog, before responding")?;
		tester.tap(&choice)?;
		let expected = match (platform, allow) {
			(Platform::Ios | Platform::Android, true) => "Notifications allowed",
			(Platform::Ios | Platform::Android, false) => "Notifications denied",
			(_, true) => "Selected: evidence.txt",
			(_, false) => "Cancelled",
		};
		tester.expect_visible(&Locator::text(expected))?;
		tester.expect_absent(&choice)?;
		tester.checkpoint("Result returned by the native dialog")?;
		// A flat placeholder image cannot satisfy the capture contract. The UI
		// assertion above separately proves this frame belongs to the scenario.
		let capture = tester.screenshot()?;
		let (width, height) = capture.dimensions();
		if width < 200 || height < 200 {
			return Err(Error::Assertion(
				"native capture is unexpectedly small".to_owned(),
			));
		}
		if capture.distinct_colors(32)? <= 32 {
			return Err(Error::Assertion(
				"capture has too few colors for rendered native text and controls".to_owned(),
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
	eprintln!("native_conformance requires --features native-ui on a desktop host");
	std::process::exit(1);
}
