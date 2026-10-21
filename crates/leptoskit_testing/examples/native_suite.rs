//! Run a real native scenario: `cargo run -p leptoskit_testing --features
//! native-ui --example native_suite -- suite.json`.
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let path = std::env::args_os()
		.nth(1)
		.ok_or("usage: native_suite <suite.json>")?;
	let suite: leptoskit_testing::native::suite::Suite =
		serde_json::from_slice(&std::fs::read(path)?)?;
	println!(
		"Native test report: {}",
		suite.run()?.join("index.html").display()
	);
	Ok(())
}

#[cfg(not(all(feature = "native-ui", not(target_arch = "wasm32"))))]
fn main() {
	eprintln!("native_suite requires --features native-ui on a desktop host");
	std::process::exit(1);
}
