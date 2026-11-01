//! Linux notifications through a real D-Bus session bus.
//!
//! The binary starts a private `dbus-daemon` and re-runs itself on that bus,
//! so a desktop's own notification server is never touched. A fake
//! `org.freedesktop.Notifications` records each `Notify` and
//! `CloseNotification` call and answers with an ID or a chosen error.

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
fn main() -> std::process::ExitCode {
	linux::main()
}

/// Lists no tests elsewhere, so every runner sees a well-formed harness.
#[cfg(not(all(target_os = "linux", not(target_env = "ohos"))))]
fn main() -> std::process::ExitCode {
	libtest_mimic::run(&libtest_mimic::Arguments::from_args(), Vec::new()).exit_code()
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod linux {
	use std::io::BufRead;
	use std::io::BufReader;
	use std::process::Command;
	use std::process::ExitCode;
	use std::process::Stdio;
	use std::sync::Mutex;
	use std::sync::OnceLock;
	use std::time::Duration;

	use dbus::arg::PropMap;
	use dbus::arg::prop_cast;
	use dbus::blocking::Connection;
	use dbus::channel::MatchingReceiver;
	use dbus::channel::Sender;
	use dbus::message::MatchRule;
	use dbus::strings::ErrorName;
	use leptoskit_bridge::Error;
	use leptoskit_plugin_notifications::Identifier;
	use leptoskit_plugin_notifications::Notification;
	use leptoskit_plugin_notifications::linux::Notifications;
	use libtest_mimic::Arguments;
	use libtest_mimic::Failed;
	use libtest_mimic::Trial;

	const PRIVATE_BUS: &str = "LEPTOSKIT_NOTIFICATIONS_PRIVATE_BUS";
	const SERVICE: &str = "org.freedesktop.Notifications";

	/// How the fake service answers the next call.
	#[derive(Clone, Copy)]
	enum Reply {
		Id(u32),
		Error,
	}

	/// A `Notify` call as the fake service received it.
	#[derive(Clone, Debug, PartialEq)]
	struct Notify {
		application: String,
		replaces: u32,
		icon: String,
		summary: String,
		body: String,
		actions: Vec<String>,
		suppress_sound: Option<bool>,
		timeout: i32,
	}

	struct Service {
		reply: Mutex<Reply>,
		notified: Mutex<Vec<Notify>>,
		closed: Mutex<Vec<u32>>,
	}

	pub(super) fn main() -> ExitCode {
		if std::env::var_os(PRIVATE_BUS).is_none() {
			return rerun_on_private_bus();
		}

		let mut arguments = Arguments::from_args();
		// Clients hold an Rc connection; one thread keeps each trial simple.
		arguments.test_threads = Some(1);
		let trials = vec![
			Trial::test("a_missing_service_fails_natively", missing_service),
			Trial::test(
				"notifications_escape_markup_and_cancel_by_id",
				shows_and_cancels,
			),
			Trial::test("service_failures_map_to_native_errors", failures),
		];
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

	/// Starts the fake service once; later trials reconfigure its reply.
	fn service() -> &'static Service {
		static STATE: OnceLock<&'static Service> = OnceLock::new();
		STATE.get_or_init(|| {
			let state: &'static Service = Box::leak(Box::new(Service {
				reply: Mutex::new(Reply::Id(1)),
				notified: Mutex::new(Vec::new()),
				closed: Mutex::new(Vec::new()),
			}));
			let (ready, started) = std::sync::mpsc::channel();
			std::thread::spawn(move || {
				let connection = Connection::new_session().expect("connect to the private bus");
				connection
					.request_name(SERVICE, false, true, true)
					.expect("own the notifications name");
				connection.start_receive(
					MatchRule::new_method_call(),
					Box::new(move |message, connection| {
						let reply = *state.reply.lock().unwrap();
						let refused = message.error(
							&ErrorName::new("org.freedesktop.DBus.Error.Failed").unwrap(),
							c"refused",
						);
						let response = match message.member().as_deref() {
							Some("Notify") => {
								let mut items = message.iter_init();
								let notify = Notify {
									application: items.read().unwrap(),
									replaces: items.read().unwrap(),
									icon: items.read().unwrap(),
									summary: items.read().unwrap(),
									body: items.read().unwrap(),
									actions: items.read().unwrap(),
									suppress_sound: prop_cast::<bool>(
										&items.read::<PropMap>().unwrap(),
										"suppress-sound",
									)
									.copied(),
									timeout: items.read().unwrap(),
								};
								state.notified.lock().unwrap().push(notify);
								match reply {
									Reply::Id(id) => message.method_return().append1(id),
									Reply::Error => refused,
								}
							}
							Some("CloseNotification") => {
								state.closed.lock().unwrap().push(message.read1().unwrap());
								match reply {
									Reply::Id(_) => message.method_return(),
									Reply::Error => refused,
								}
							}
							_ => {
								message.error(
									&ErrorName::new("org.freedesktop.DBus.Error.UnknownMethod")
										.unwrap(),
									c"unknown",
								)
							}
						};

						connection.send(response).expect("send the reply");
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
			started.recv().expect("the fake service starts");
			state
		})
	}

	fn notification(body: &str) -> Notification {
		Notification::new(
			Identifier::new("order-1").unwrap(),
			"Order shipped",
			body,
			Identifier::new("orders").unwrap(),
		)
		.unwrap()
	}

	fn missing_service() -> Result<(), Failed> {
		// Runs before any trial starts the fake service.
		for application in ["", " ", "Leptos\u{7}Kit", &"a".repeat(257)] {
			if !matches!(Notifications::new(application), Err(Error::InvalidArgument)) {
				return Err(format!("accepted application name {application:?}").into());
			}
		}

		let notifications =
			Notifications::new("LeptosKit test").map_err(|error| format!("{error:?}"))?;

		match notifications.show(&notification("no server")) {
			Err(Error::Native) => Ok(()),
			Err(error) => Err(format!("expected Native, got {error:?}").into()),
			Ok(_) => Err("showed a notification without a notification server".into()),
		}
	}

	fn shows_and_cancels() -> Result<(), Failed> {
		let service = service();
		*service.reply.lock().unwrap() = Reply::Id(42);
		service.notified.lock().unwrap().clear();
		service.closed.lock().unwrap().clear();
		let notifications =
			Notifications::new("LeptosKit test").map_err(|error| format!("{error:?}"))?;
		let handle = notifications
			.show(&notification("<b>Fast</b> & free\n<a href=\"x\">track</a>"))
			.map_err(|error| format!("show: {error:?}"))?;
		assert_eq!(
			*service.notified.lock().unwrap(),
			[Notify {
				application: "LeptosKit test".to_owned(),
				replaces: 0,
				icon: String::new(),
				summary: "Order shipped".to_owned(),
				body: "&lt;b&gt;Fast&lt;/b&gt; &amp; free\n&lt;a href=\"x\"&gt;track&lt;/a&gt;"
					.to_owned(),
				actions: Vec::new(),
				suppress_sound: Some(true),
				timeout: -1,
			}]
		);
		handle
			.cancel()
			.map_err(|error| format!("cancel: {error:?}"))?;

		assert_eq!(*service.closed.lock().unwrap(), [42]);
		Ok(())
	}

	fn failures() -> Result<(), Failed> {
		let service = service();
		let notifications =
			Notifications::new("LeptosKit test").map_err(|error| format!("{error:?}"))?;
		// A zero ID is reserved by the specification and never a valid handle.
		*service.reply.lock().unwrap() = Reply::Id(0);
		assert_eq!(
			notifications.show(&notification("zero")).err(),
			Some(Error::Native)
		);
		*service.reply.lock().unwrap() = Reply::Error;
		assert_eq!(
			notifications.show(&notification("refused")).err(),
			Some(Error::Native)
		);
		*service.reply.lock().unwrap() = Reply::Id(7);
		let handle = notifications
			.show(&notification("dismissed elsewhere"))
			.map_err(|error| format!("show: {error:?}"))?;
		*service.reply.lock().unwrap() = Reply::Error;
		assert_eq!(handle.cancel(), Err(Error::Native));
		*service.reply.lock().unwrap() = Reply::Id(1);
		Ok(())
	}
}
