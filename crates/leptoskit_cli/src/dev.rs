//! `leptoskit dev`: watch, rebuild, and serve, with short feedback loops.

use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use notify::Watcher;

use crate::cli::REBUILD_DEBOUNCE;

/// Runs the dev loop: an initial build, then a watcher that rebuilds after
/// changes settle, with the static server up throughout.
pub fn run(open: bool, port: Option<u16>) -> Result<()> {
	let (config, root) =
		leptoskit_config::LeptoskitConfig::load().context("no leptoskit project found")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let port = port.unwrap_or(config.serve.port);

	crate::build::run(false, None, "dist")?;

	let (events_tx, events_rx) = mpsc::channel::<()>();
	let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
		if event
			.ok()
			.is_some_and(|event| event.paths.iter().any(|path| is_relevant(path)))
		{
			let _ = events_tx.send(());
		}
	})
	.context("could not start the file watcher")?;
	watcher
		.watch(
			root.join("src").as_std_path(),
			notify::RecursiveMode::Recursive,
		)
		.context("could not watch src/")?;
	let public = root.join("public");
	if public.is_dir() {
		watcher
			.watch(public.as_std_path(), notify::RecursiveMode::Recursive)
			.context("could not watch public/")?;
	}

	println!("watching {} for changes", root.join("src"));
	if open {
		let _ = std::process::Command::new("open")
			.arg(format!("http://127.0.0.1:{port}"))
			.spawn();
	}

	let server = std::thread::spawn(move || {
		let runtime = tokio::runtime::Runtime::new().expect("async runtime");
		let options = leptoskit_axum::ServeOptions::new("dist", "127.0.0.1", port);
		runtime.block_on(async move {
			let _ = leptoskit_axum::serve(options).await;
		});
	});

	loop {
		if events_rx.recv().is_err() {
			break;
		}
		// Debounce: wait for the editor's burst of writes to settle.
		let settled_at = Instant::now();
		while Instant::now().duration_since(settled_at) < REBUILD_DEBOUNCE {
			if events_rx.try_recv().is_err() {
				std::thread::sleep(Duration::from_millis(25));
			}
		}
		let started = Instant::now();
		match crate::build::run(false, None, "dist") {
			Ok(()) => println!("rebuilt in {:?}", started.elapsed()),
			Err(error) => eprintln!("rebuild failed: {error:#}"),
		}
	}
	let _ = server.join();
	Ok(())
}

/// Which watched paths trigger a rebuild.
fn is_relevant(path: &std::path::Path) -> bool {
	matches!(
		path.extension().and_then(|extension| extension.to_str()),
		Some("rs" | "toml" | "css" | "html" | "js" | "wasm")
	)
}
