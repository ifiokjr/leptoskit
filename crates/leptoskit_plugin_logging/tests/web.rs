#![cfg(all(target_arch = "wasm32", not(feature = "guest")))]

use std::collections::BTreeMap;

use leptoskit_bridge::Error;
use leptoskit_plugin_logging::Level;
use leptoskit_plugin_logging::Record;
use leptoskit_plugin_logging::web::Logger;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r"
let original;
let records;
export function captureLoggingConsole() {
  records = [];
  original = Object.fromEntries(['debug','info','warn','error'].map(name => [name,console[name]]));
  for (const name of Object.keys(original)) console[name] = value => records.push({method:name,value});
}
export function restoreLoggingConsole() { Object.assign(console, original); }
export function loggingRecords() { return JSON.stringify(records); }
")]
extern "C" {
	#[wasm_bindgen(js_name = captureLoggingConsole)]
	fn capture();
	#[wasm_bindgen(js_name = restoreLoggingConsole)]
	fn restore();
	#[wasm_bindgen(js_name = loggingRecords)]
	fn records() -> String;
}
struct Capture;
impl Drop for Capture {
	fn drop(&mut self) {
		restore();
	}
}

#[wasm_bindgen_test]
fn browser_emits_structured_objects_at_expected_levels_and_stops_after_close() {
	capture();
	let _capture = Capture;
	let mut logger = Logger::new(Level::Info);
	let mut record = Record {
		level: Level::Debug,
		target: "tests".into(),
		message: "synthetic".into(),
		fields: BTreeMap::from([("case".into(), "browser".into())]),
	};
	assert_eq!(logger.write(&record), Ok(false));

	for level in [Level::Info, Level::Warn, Level::Error] {
		record.level = level;
		assert_eq!(logger.write(&record), Ok(true));
	}

	logger.flush().unwrap();
	logger.close();
	assert_eq!(logger.write(&record), Err(Error::Cancelled));
	assert_eq!(logger.flush(), Err(Error::Cancelled));

	logger.close();
	let emitted: serde_json::Value = serde_json::from_str(&records()).unwrap();
	let emitted = emitted.as_array().unwrap();
	assert_eq!(emitted.len(), 3);

	for (entry, level) in emitted.iter().zip(["info", "warn", "error"]) {
		assert_eq!(entry["method"], level);
		assert_eq!(entry["value"]["level"], level);
		assert_eq!(entry["value"]["fields"]["case"], "browser");
		assert!(entry["value"]["timestamp_ms"].as_u64().unwrap() > 0);
	}
}
