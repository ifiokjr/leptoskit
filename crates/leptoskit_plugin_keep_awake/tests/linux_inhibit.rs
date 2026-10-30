//! Linux display leases through a real D-Bus session bus.
//!
//! The binary starts a private `dbus-daemon` and re-runs itself on that bus,
//! so a desktop's own screensaver service is never touched. A fake
//! `org.freedesktop.ScreenSaver` answers `Inhibit` with a cookie or a chosen
//! error, and the test observes each lease's private connection close.

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
	use std::time::Instant;

	use dbus::Message;
	use dbus::blocking::Connection;
	use dbus::channel::MatchingReceiver;
	use dbus::channel::Sender;
	use dbus::message::MatchRule;
	use dbus::strings::ErrorName;
	use leptoskit_bridge::Error;
	use leptoskit_plugin_keep_awake::desktop::Lease;
	use libtest_mimic::Arguments;
	use libtest_mimic::Failed;
	use libtest_mimic::Trial;

	const PRIVATE_BUS: &str = "LEPTOSKIT_KEEP_AWAKE_PRIVATE_BUS";
	const SERVICE: &str = "org.freedesktop.ScreenSaver";

	/// How the fake service answers the next `Inhibit` call.
	#[derive(Clone, Copy)]
	enum Reply {
		Cookie,
		Error(&'static str),
	}

	/// An `Inhibit` call the fake service accepted.
	#[derive(Clone)]
	struct Inhibition {
		sender: String,
		application: String,
		reason: String,
	}

	struct Service {
		reply: Mutex<Reply>,
		calls: Mutex<Vec<Inhibition>>,
	}

	pub(super) fn main() -> ExitCode {
		if std::env::var_os(PRIVATE_BUS).is_none() {
			return rerun_on_private_bus();
		}
		let mut arguments = Arguments::from_args();
		// Leases are thread-local; one thread keeps each trial's view simple.
		arguments.test_threads = Some(1);
		let trials = vec![
			Trial::test("a_missing_service_is_unavailable", missing_service),
			Trial::test(
				"leases_share_one_inhibitor_held_by_their_connection",
				shared,
			),
			Trial::test("service_errors_map_to_bridge_statuses", errors),
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
		static SERVICE_STATE: OnceLock<&'static Service> = OnceLock::new();
		SERVICE_STATE.get_or_init(|| {
			let state: &'static Service = Box::leak(Box::new(Service {
				reply: Mutex::new(Reply::Cookie),
				calls: Mutex::new(Vec::new()),
			}));
			let (ready, started) = std::sync::mpsc::channel();
			std::thread::spawn(move || {
				let connection = Connection::new_session().expect("connect to the private bus");
				connection
					.request_name(SERVICE, false, true, true)
					.expect("own the screensaver name");
				connection.start_receive(
					MatchRule::new_method_call(),
					Box::new(move |message, connection| {
						connection
							.send(answer(state, &message))
							.expect("send the reply");
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

	/// The fake service's reply to one method call: a cookie or the chosen
	/// error for `Inhibit`, and an unknown-method error for anything else.
	fn answer(state: &Service, message: &Message) -> Message {
		let inhibit = message.interface().as_deref() == Some(SERVICE)
			&& message.member().as_deref() == Some("Inhibit");
		if !inhibit {
			return message.error(
				&ErrorName::new("org.freedesktop.DBus.Error.UnknownMethod").unwrap(),
				c"unknown",
			);
		}
		let (application, reason): (String, String) = message.read2().expect("Inhibit arguments");
		match *state.reply.lock().unwrap() {
			Reply::Cookie => {
				state.calls.lock().unwrap().push(Inhibition {
					sender: message.sender().unwrap().to_string(),
					application,
					reason,
				});
				message.method_return().append1(7_u32)
			}
			Reply::Error(name) => message.error(&ErrorName::new(name).unwrap(), c"refused"),
		}
	}

	/// Whether a connection's unique name is still on the bus.
	fn connected(name: &str) -> bool {
		let connection = Connection::new_session().unwrap();
		let proxy = connection.with_proxy(
			"org.freedesktop.DBus",
			"/org/freedesktop/DBus",
			Duration::from_secs(5),
		);
		let (owned,): (bool,) = proxy
			.method_call("org.freedesktop.DBus", "NameHasOwner", (name,))
			.unwrap();
		owned
	}

	fn disconnects(name: &str) -> bool {
		let deadline = Instant::now() + Duration::from_secs(5);
		while Instant::now() < deadline {
			if !connected(name) {
				return true;
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		false
	}

	fn missing_service() -> Result<(), Failed> {
		// Runs before any trial starts the fake service.
		if Connection::new_session()?
			.with_proxy("org.freedesktop.DBus", "/", Duration::from_secs(5))
			.method_call::<(bool,), _, _, _>("org.freedesktop.DBus", "NameHasOwner", (SERVICE,))?
			.0
		{
			return Err("the screensaver name is already owned on the private bus".into());
		}
		match Lease::acquire("LeptosKit test", "no screensaver service") {
			Err(Error::Unavailable) => Ok(()),
			Err(error) => Err(format!("expected Unavailable, got {error:?}").into()),
			Ok(_) => Err("acquired a lease without a screensaver service".into()),
		}
	}

	fn shared() -> Result<(), Failed> {
		let service = service();
		*service.reply.lock().unwrap() = Reply::Cookie;
		service.calls.lock().unwrap().clear();
		let first = Lease::acquire("LeptosKit test", "keep the display on")
			.map_err(|error| format!("acquire: {error:?}"))?;
		let second = Lease::acquire("LeptosKit test", "shares the first inhibitor")
			.map_err(|error| format!("acquire: {error:?}"))?;
		assert_eq!(second.owners(), 2);
		let calls = service.calls.lock().unwrap().clone();
		assert_eq!(
			calls.len(),
			1,
			"the second lease shares the first inhibitor"
		);
		assert_eq!(calls[0].application, "LeptosKit test");
		assert_eq!(calls[0].reason, "keep the display on");
		let holder = calls[0].sender.clone();
		assert!(connected(&holder));
		drop(first);
		assert_eq!(second.owners(), 1);
		assert!(
			connected(&holder),
			"an earlier owner's drop keeps the inhibitor"
		);
		drop(second);
		assert!(
			disconnects(&holder),
			"the final owner's drop closes the inhibiting connection"
		);
		let again = Lease::acquire("LeptosKit test", "a fresh inhibitor")
			.map_err(|error| format!("acquire: {error:?}"))?;
		assert_eq!(again.owners(), 1);
		assert_eq!(service.calls.lock().unwrap().len(), 2);
		Ok(())
	}

	fn errors() -> Result<(), Failed> {
		let service = service();
		for (name, expected) in [
			(
				"org.freedesktop.DBus.Error.AccessDenied",
				Error::PermissionDenied,
			),
			(
				"org.freedesktop.DBus.Error.UnknownMethod",
				Error::Unavailable,
			),
			(
				"org.freedesktop.DBus.Error.ServiceUnknown",
				Error::Unavailable,
			),
			("org.example.Error.Failed", Error::Native),
		] {
			*service.reply.lock().unwrap() = Reply::Error(name);
			let result = Lease::acquire("LeptosKit test", "refused inhibitor").map(|_| ());
			if result != Err(expected) {
				return Err(format!("{name}: expected {expected:?}, got {result:?}").into());
			}
		}
		*service.reply.lock().unwrap() = Reply::Cookie;
		Ok(())
	}
}
