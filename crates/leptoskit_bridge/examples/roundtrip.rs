//! Diagnostic microbenchmark. Run in release mode on an otherwise idle host.
//! Includes submission, host dequeue, completion, and consumption. It excludes
//! wasm/JNI, scheduling, and OS operations; never use it for framework
//! comparisons.

use std::hint::black_box;
use std::time::Instant;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;

fn main() {
	for size in [0, 64, 8192, 1024 * 1024] {
		let mut samples = Vec::new();
		let mut bridge = Bridge::default();
		let plugin = bridge.register("echo", [MethodId(0)]).expect("register");
		bridge.grant(plugin, MethodId(0), true).expect("grant");
		let payload = Bytes::from(vec![42; size]);
		let iterations = 100_000_u32;

		for _ in 0..5 {
			let start = Instant::now();

			for _ in 0..iterations {
				let id = bridge
					.submit(plugin, MethodId(0), black_box(payload.clone()))
					.expect("submit");
				let request = bridge.next_request().expect("request");
				assert_eq!(request.payload.len(), size);
				assert_eq!(request.payload.as_ptr(), payload.as_ptr());
				bridge.complete(id, Ok(request.payload)).expect("complete");
				let response = bridge
					.take_result(id)
					.expect("call")
					.expect("ready")
					.expect("success");
				assert_eq!(response.len(), size);
				assert_eq!(response.as_ptr(), payload.as_ptr());
				black_box(response);
			}

			samples.push(start.elapsed().as_secs_f64() * 1e9 / f64::from(iterations));
		}

		samples.sort_by(f64::total_cmp);
		println!(
			"{size} bytes: median {:.1} ns/roundtrip, range {:.1}..{:.1}; 5 x {iterations} \
			 successful roundtrips; 0 payload copies; assertions included",
			samples[2], samples[0], samples[4]
		);
	}
}
