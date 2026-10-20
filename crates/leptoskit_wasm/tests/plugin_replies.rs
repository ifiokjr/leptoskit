use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Encoder;
use leptoskit_native::EventOutcome;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

#[test]
#[ignore = "run test:plugins:guest to compile and load the Rust wasm fixture"]
fn compiled_rust_guest_validates_plugin_acknowledgements_and_batch_cardinality() {
	let path = std::env::var("LEPTOSKIT_PLUGIN_REPLY_GUEST").expect("fixture wasm path");
	let mut host = WasmHost::headless(&std::fs::read(path).unwrap()).unwrap();
	for (name, count) in [("filesystem", 7), ("haptics", 4), ("sqlite", 4)] {
		let plugin = host
			.plugins()
			.register(name, (0..count).map(MethodId))
			.unwrap();
		for method in 0..count {
			host.plugins()
				.grant(plugin, MethodId(method), true)
				.unwrap();
		}
	}
	host.start(100.0, 100.0).unwrap();
	assert_eq!(
		host.dispatch_wire(0, EventKind::Press, None).unwrap(),
		EventOutcome::Unhandled
	);
	// Filesystem mutations, haptic effects, then a two-statement transaction.
	let calls = [
		(0, 1),
		(0, 3),
		(0, 4),
		(0, 6),
		(1, 0),
		(1, 1),
		(1, 2),
		(1, 3),
		(2, 2),
	];
	for valid in [false, true] {
		for (plugin, method) in calls {
			let request = host
				.plugins()
				.next_request()
				.expect("guest must submit the next operation");
			assert_eq!((request.plugin.0, request.method.0), (plugin, method));
			let response = if plugin == 2 {
				let count = if valid { 2 } else { 1 };
				let mut encoder = Encoder::default();
				encoder.u32(count);
				// Each SQLite result contains changes u64 LE and rowid i64 LE.
				let mut result = [0; 16];
				result[..8].copy_from_slice(&1_u64.to_le_bytes());
				result[8..].copy_from_slice(&2_i64.to_le_bytes());
				for _ in 0..count {
					encoder.bytes(&result).unwrap();
				}
				encoder.finish()
			} else if valid {
				Bytes::new()
			} else {
				Bytes::from_static(&[42])
			};
			host.plugins().complete(request.id, Ok(response)).unwrap();
			host.wake_plugins().expect("guest assertions must pass");
		}
	}
	assert!(host.plugins().next_request().is_none());
	assert_eq!(host.plugins().buffered_bytes(), 0);
	assert_eq!(
		host.dispatch_wire(0, EventKind::Press, None).unwrap(),
		EventOutcome::Handled
	);
}
