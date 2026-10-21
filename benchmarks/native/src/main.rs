//! `leptoskit_bench`: the native, wasm, and JavaScript layer benchmarks in
//! one binary, so the same measurement runs on the host, in the iOS
//! simulator, and on an Android emulator.
//!
//! The output is a `BenchmarkRun` JSON document (the browser harness's
//! schema), written to `--out` or, for device runs where no file comes back,
//! printed to stdout between `LEPTOSKIT_BENCH_JSON_BEGIN` and
//! `LEPTOSKIT_BENCH_JSON_END` lines for the harness to harvest.
//!
//! ```text
//! leptoskit_bench [--out run.json] [--runs 15] [--warmup 2] [--subject native-host] [--only native|yoga|wasm|js]
//! ```

#[cfg(feature = "js")]
mod js_layer;
mod report;
mod view;
mod wasm_layer;

include!(concat!(env!("OUT_DIR"), "/embedded_guest.rs"));

use report::Config;
use report::Report;
use serde::Serialize;

struct Arguments {
	out: Option<String>,
	runs: usize,
	warmup: usize,
	subject: String,
	only: Option<String>,
}

fn parse_arguments() -> Result<Arguments, String> {
	let mut arguments = Arguments {
		out: None,
		runs: 15,
		warmup: 2,
		subject: format!("native-{}", std::env::consts::OS),
		only: None,
	};
	let mut raw = std::env::args().skip(1);
	while let Some(flag) = raw.next() {
		let mut value = || raw.next().ok_or_else(|| format!("{flag} needs a value"));
		match flag.as_str() {
			"--out" => arguments.out = Some(value()?),
			"--runs" => {
				arguments.runs = value()?
					.parse()
					.map_err(|error| format!("--runs: {error}"))?;
			}
			"--warmup" => {
				arguments.warmup = value()?
					.parse()
					.map_err(|error| format!("--warmup: {error}"))?;
			}
			"--subject" => arguments.subject = value()?,
			"--only" => arguments.only = Some(value()?),
			other => return Err(format!("unknown argument {other}")),
		}
	}
	Ok(arguments)
}

fn wants(only: Option<&str>, layer: &str) -> bool {
	only.is_none_or(|filter| filter.split(',').any(|item| item.trim() == layer))
}

fn run() -> Result<(), String> {
	let arguments = parse_arguments()?;
	let config = Config {
		runs: arguments.runs,
		warmup: arguments.warmup,
	};
	let only = arguments.only.as_deref();
	let mut report = Report::default();

	if wants(only, "native") || wants(only, "yoga") {
		eprintln!("[bench] view layer");
		view::run(&mut report, config)?;
	}
	if wants(only, "wasm") {
		eprintln!("[bench] wasm layer");
		wasm_layer::run(&mut report, config, GUEST_WASM)?;
	}
	if wants(only, "js") {
		eprintln!("[bench] javascript layer");
		#[cfg(feature = "js")]
		js_layer::run(&mut report, config)?;
		#[cfg(not(feature = "js"))]
		report.note("javascript layer skipped: built without the `js` feature");
	}

	let document = report.to_json(&arguments.subject, config);
	let mut json = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut serializer = serde_json::Serializer::with_formatter(&mut json, formatter);
	document
		.serialize(&mut serializer)
		.map_err(|error| format!("serialize report: {error}"))?;
	let json = String::from_utf8(json).map_err(|error| format!("serialize report: {error}"))?;
	if let Some(path) = arguments.out {
		std::fs::write(&path, format!("{json}\n"))
			.map_err(|error| format!("write {path}: {error}"))?;
		eprintln!("[bench] wrote {path}");
	} else {
		println!("LEPTOSKIT_BENCH_JSON_BEGIN");
		println!("{json}");
		println!("LEPTOSKIT_BENCH_JSON_END");
	}
	Ok(())
}

fn main() {
	if let Err(error) = run() {
		eprintln!("leptoskit_bench: {error}");
		std::process::exit(1);
	}
}
