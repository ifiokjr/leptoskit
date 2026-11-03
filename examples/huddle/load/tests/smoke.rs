//! The load tool must speak the same protocol as Huddle and account for
//! every delivered update, even when clients are unevenly split over rooms.

use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simulated_clients_receive_every_committed_message() {
	let db = leptoskit_db::testing::sqlite_migrated(huddle_db::MIGRATIONS).await;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let mut options = huddle_server::Options::new(&db);
	options.origin = Some(base.clone());
	let auth = leptoskit_auth::Auth::builder(&db, leptoskit_auth::Keys::single("test", [3; 32]))
		.origin(&base);
	let huddle = huddle_server::start(&db, auth, options).unwrap();
	let router = huddle.router.clone();
	let server = tokio::spawn(async move {
		axum::serve(listener, router).await.unwrap();
	});
	let output = tokio::time::timeout(
		Duration::from_secs(40),
		tokio::process::Command::new(env!("CARGO_BIN_EXE_huddle-load"))
			.args([
				"--url",
				&base,
				"--clients",
				"3",
				"--channels",
				"2",
				"--accounts",
				"2",
				"--seconds",
				"1",
				"--rate",
				"4",
				"--connect-concurrency",
				"2",
			])
			.kill_on_drop(true)
			.output(),
	)
	.await
	.expect("load run completes")
	.unwrap();
	huddle.shutdown().await;
	server.abort();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let report = String::from_utf8(output.stdout).unwrap();
	let metric = |label: &str| -> u64 {
		report
			.lines()
			.find_map(|line| line.strip_prefix(label))
			.unwrap_or_else(|| panic!("missing {label}: {report}"))
			.trim()
			.parse()
			.unwrap()
	};
	let sent = metric("messages sent");
	let expected = metric("updates expected");
	assert!(sent >= 4, "{report}");
	assert!(
		expected > sent,
		"some rooms have multiple subscribers: {report}"
	);
	assert_eq!(metric("updates received"), expected, "{report}");
	assert_eq!(metric("errors"), 0, "{report}");
}
