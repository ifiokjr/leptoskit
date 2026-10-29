//! Linux tray lifecycle through a real D-Bus session bus.
//!
//! The binary starts a private `dbus-daemon` and re-runs itself on that bus,
//! so a desktop's own tray host is never touched. A fake
//! `org.kde.StatusNotifierWatcher` accepts the item's registration, and the
//! trial reads the exported `StatusNotifierItem` properties back over the bus.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
	linux::main()
}

/// Lists no tests elsewhere, so every runner sees a well-formed harness.
#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
	libtest_mimic::run(&libtest_mimic::Arguments::from_args(), Vec::new()).exit_code()
}

#[cfg(target_os = "linux")]
mod linux {
	use std::io::BufRead;
	use std::io::BufReader;
	use std::process::Command;
	use std::process::ExitCode;
	use std::process::Stdio;
	use std::rc::Rc;
	use std::sync::Mutex;
	use std::time::Duration;
	use std::time::Instant;

	use dbus::arg::Variant;
	use dbus::blocking::Connection;
	use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
	use dbus::channel::MatchingReceiver;
	use dbus::channel::Sender;
	use dbus::message::MatchRule;
	use dbus::strings::ErrorName;
	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::DesktopMenu;
	use leptoskit_plugin_menu::Item;
	use leptoskit_plugin_tray::Error;
	use leptoskit_plugin_tray::Event;
	use leptoskit_plugin_tray::Options;
	use leptoskit_plugin_tray::RgbaIcon;
	use leptoskit_plugin_tray::TrayHost;
	use libtest_mimic::Arguments;
	use libtest_mimic::Failed;
	use libtest_mimic::Trial;

	const PRIVATE_BUS: &str = "LEPTOSKIT_TRAY_PRIVATE_BUS";
	const WATCHER: &str = "org.kde.StatusNotifierWatcher";
	const ITEM: &str = "org.kde.StatusNotifierItem";

	/// Item services the fake watcher accepted, in registration order.
	static REGISTERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

	pub(super) fn main() -> ExitCode {
		if std::env::var_os(PRIVATE_BUS).is_none() {
			return rerun_on_private_bus();
		}
		let mut arguments = Arguments::from_args();
		// One tray host per process: the trials share it in order.
		arguments.test_threads = Some(1);
		let trials = vec![Trial::test(
			"tray_lifecycle_updates_the_registered_status_notifier_item",
			lifecycle,
		)];
		libtest_mimic::run(&arguments, trials).exit_code()
	}

	fn rerun_on_private_bus() -> ExitCode {
		let mut daemon = Command::new("dbus-daemon")
			.args(["--session", "--nofork", "--print-address=1"])
			.stdout(Stdio::piped())
			.spawn()
			.expect("dbus-daemon is installed (the dbus package provides it)");
		let mut address = String::new();
		BufReader::new(daemon.stdout.take().expect("piped stdout"))
			.read_line(&mut address)
			.expect("dbus-daemon prints its address");
		let status = Command::new(std::env::current_exe().expect("test binary path"))
			.args(std::env::args_os().skip(1))
			.env("DBUS_SESSION_BUS_ADDRESS", address.trim())
			.env(PRIVATE_BUS, "1")
			.status()
			.expect("re-run the tests on the private bus");
		daemon.kill().expect("stop the private dbus-daemon");
		daemon.wait().expect("reap the private dbus-daemon");
		match status.code() {
			Some(0) => ExitCode::SUCCESS,
			_ => ExitCode::FAILURE,
		}
	}

	/// Serves the watcher methods and properties ksni uses to register.
	fn start_watcher() {
		let (ready, started) = std::sync::mpsc::channel();
		std::thread::spawn(move || {
			let connection = Connection::new_session().expect("connect to the private bus");
			connection
				.request_name(WATCHER, false, true, true)
				.expect("own the watcher name");
			connection.start_receive(
				MatchRule::new_method_call(),
				Box::new(|message, connection| {
					let member = message.member().map(|member| member.to_string());
					let reply = match member.as_deref() {
						Some("RegisterStatusNotifierItem") => {
							let service: String = message.read1().expect("item service");
							REGISTERED.lock().unwrap().push(service);
							message.method_return()
						}
						Some("Get") => {
							let (_, property): (String, String) =
								message.read2().expect("property request");
							match property.as_str() {
								"IsStatusNotifierHostRegistered" => {
									message.method_return().append1(Variant(true))
								}
								"ProtocolVersion" => {
									message.method_return().append1(Variant(0_i32))
								}
								_ => {
									message.error(
										&ErrorName::new(
											"org.freedesktop.DBus.Error.UnknownProperty",
										)
										.unwrap(),
										c"unknown property",
									)
								}
							}
						}
						_ => {
							message.error(
								&ErrorName::new("org.freedesktop.DBus.Error.UnknownMethod")
									.unwrap(),
								c"unknown method",
							)
						}
					};
					connection.send(reply).expect("send the reply");
					true
				}),
			);
			ready.send(()).unwrap();
			loop {
				connection
					.process(Duration::from_secs(1))
					.expect("serve the private bus");
			}
		});
		started.recv().expect("the fake watcher starts");
	}

	/// Polls one exported item property until it matches or five seconds pass.
	fn wait_for_property(service: &str, property: &str, expected: &str) -> Result<(), Failed> {
		let connection = Connection::new_session()?;
		let proxy = connection.with_proxy(service, "/StatusNotifierItem", Duration::from_secs(5));
		let deadline = Instant::now() + Duration::from_secs(5);
		loop {
			let value: String = proxy.get(ITEM, property)?;
			if value == expected {
				return Ok(());
			}
			if Instant::now() >= deadline {
				return Err(format!("{property} stayed {value:?}, expected {expected:?}").into());
			}
			std::thread::sleep(Duration::from_millis(20));
		}
	}

	fn icon() -> RgbaIcon {
		RgbaIcon::new(vec![255, 0, 0, 255], 1, 1).unwrap()
	}

	fn lifecycle() -> Result<(), Failed> {
		start_watcher();
		let mut host = TrayHost::install(|| {}).map_err(|error| format!("{error:?}"))?;
		let menu = Rc::new(
			DesktopMenu::new(Definition::new(vec![Item::Command {
				id: "open".into(),
				label: "Open".into(),
				enabled: true,
				accelerator: None,
			}])?)
			.map_err(|error| format!("{error:?}"))?,
		);
		assert!(matches!(
			host.create(
				icon(),
				&Options {
					template: true,
					..Options::default()
				},
				None
			),
			Err(Error::Unavailable)
		));
		let id = host
			.create(
				icon(),
				&Options {
					tooltip: Some("Tip".into()),
					title: Some("Title".into()),
					template: false,
				},
				Some(Rc::clone(&menu)),
			)
			.map_err(|error| format!("create: {error:?}"))?;
		let service = REGISTERED
			.lock()
			.unwrap()
			.last()
			.cloned()
			.ok_or("the item registered with the watcher")?;
		wait_for_property(&service, "Title", "Title")?;
		wait_for_property(&service, "Status", "Active")?;

		assert!(matches!(
			host.create(icon(), &Options::default(), None),
			Err(Error::Capacity)
		));
		host.set_title(id, Some("Renamed"))
			.map_err(|error| format!("{error:?}"))?;
		wait_for_property(&service, "Title", "Renamed")?;
		host.set_visible(id, false)
			.map_err(|error| format!("{error:?}"))?;
		wait_for_property(&service, "Status", "Passive")?;
		host.set_tooltip(id, Some("Hover"))
			.map_err(|error| format!("{error:?}"))?;
		assert!(matches!(
			host.set_icon(id, icon(), true),
			Err(Error::Unavailable)
		));
		host.set_icon(id, icon(), false)
			.map_err(|error| format!("{error:?}"))?;
		host.set_menu(id, None)
			.map_err(|error| format!("{error:?}"))?;
		host.set_menu(id, Some(menu))
			.map_err(|error| format!("{error:?}"))?;
		host.refresh_menu(id)
			.map_err(|error| format!("{error:?}"))?;
		assert!(matches!(host.bounds(id), Ok(None)));
		assert_eq!(host.drain().events, [] as [Event; 0]);

		host.remove(id).map_err(|error| format!("{error:?}"))?;
		assert!(matches!(host.remove(id), Err(Error::UnknownTray)));
		assert!(matches!(host.set_title(id, None), Err(Error::UnknownTray)));
		Ok(())
	}
}
