//! Steady-state Wasm guest -> C ABI echo -> guest benchmark.
//!
//! Run `cargo bench -p leptoskit_bridge_ffi --features app --bench
//! native_roundtrip`. Includes guest submission, both Wasm memory copies,
//! native request ownership, completion, fuel-metered wake, guest consumption,
//! and length/sentinel checks. Excludes initialization, OS work, Swift/JNI
//! dispatch, and UI scheduling. This is a transport measurement, not an
//! application latency comparison.
//! If macOS 27 rejects a stripped proc-macro dylib's LINKEDIT section, pass
//! `--config 'profile.release.build-override.strip="none"'` to Cargo.
#![allow(unsafe_code)]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::fmt::Write;
use std::hint::black_box;
use std::mem::MaybeUninit;
use std::time::Instant;

use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge_ffi::NativeRequest;
use leptoskit_bridge_ffi::lk_bridge_complete;
use leptoskit_bridge_ffi::lk_bridge_complete_owned;
use leptoskit_bridge_ffi::lk_bridge_next_request;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

// Clippy on Android misreads the already-const initializer below as one
// that could be const; the expectation lapses, and warns, once it stops. A
// lint attribute cannot sit on the macro call itself, so a module holds it.
#[cfg_attr(
	target_os = "android",
	expect(
		clippy::missing_const_for_thread_local,
		reason = "a false positive on Android; the initializer is const"
	)
)]
mod counters {
	use std::cell::Cell;

	thread_local! {
		pub(super) static ALLOCATIONS: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
	}
}

use counters::ALLOCATIONS;

struct CountingAllocator;

fn count(size: usize) {
	let _ = ALLOCATIONS.try_with(|count| {
		if let Some((calls, bytes)) = count.get() {
			count.set(Some((
				calls + 1,
				bytes + u64::try_from(size).expect("allocation size"),
			)));
		}
	});
}

// SAFETY: All allocator operations forward the original contract to System;
// thread-local bookkeeping uses only non-allocating Cell operations.
unsafe impl GlobalAlloc for CountingAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		count(layout.size());
		// SAFETY: Forward the caller's allocation contract unchanged.
		unsafe { System.alloc(layout) }
	}

	unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
		count(layout.size());
		// SAFETY: Forward the caller's allocation contract unchanged.
		unsafe { System.alloc_zeroed(layout) }
	}

	unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
		count(size);
		// SAFETY: Forward the caller's reallocation contract unchanged.
		unsafe { System.realloc(pointer, layout, size) }
	}

	unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
		// SAFETY: Forward the original allocation to its allocator.
		unsafe { System.dealloc(pointer, layout) }
	}
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy)]
enum Completion {
	Owned,
	Copied,
}

impl Completion {
	fn name(self) -> &'static str {
		match self {
			Self::Owned => "owned",
			Self::Copied => "copied",
		}
	}
}

fn guest(size: usize, exhaustive: bool) -> String {
	let destination = size.max(1);
	let pages = (destination + size).div_ceil(65_536).max(1);
	let clear = if exhaustive {
		format!("(memory.fill (i32.const {destination}) (i32.const 0) (i32.const {size}))")
	} else {
		String::new()
	};

	let mut sentinels = String::new();
	let mut clear_sentinels = String::new();

	if size > 0 {
		for index in [0, size / 2, size - 1] {
			let address = destination + index;
			write!(
				sentinels,
				"(if (i32.ne (i32.load8_u (i32.const {address})) (i32.const 42)) (then \
				 unreachable))"
			)
			.expect("write guest check");
			write!(
				clear_sentinels,
				"(i32.store8 (i32.const {address}) (i32.const 0))"
			)
			.expect("write guest reset");
		}
	}
	let exhaustive_check = if exhaustive && size > 0 {
		format!(
			r"
			(local.set $index (i32.const 0))
			(loop $check
				(if (i32.ne
					(i32.load8_u (i32.add (i32.const {destination}) (local.get $index)))
					(i32.const 42)) (then unreachable))
				(local.set $index (i32.add (local.get $index) (i32.const 1)))
				(br_if $check (i32.lt_u (local.get $index) (i32.const {size}))))"
		)
	} else {
		String::new()
	};

	format!(
		r#"(module
			(import "leptoskit_plugins_v1" "submit"
				(func $submit (param i32 i32 i32 i32) (result i64)))
			(import "leptoskit_plugins_v1" "take_result"
				(func $take (param i64 i32 i32) (result i64)))
			(memory (export "memory") {pages})
			(global $call (mut i64) (i64.const 0))
			(func $send
				(global.set $call (call $submit (i32.const 0) (i32.const 0)
					(i32.const 0) (i32.const {size})))
				(if (i64.le_s (global.get $call) (i64.const 0)) (then unreachable)))
			(func (export "leptoskit_main")
				(memory.fill (i32.const 0) (i32.const 42) (i32.const {size}))
				(call $send))
			(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
			(func (export "leptoskit_plugins_ready"))
			(func (export "leptoskit_poll") (local $index i32)
				{clear}
				{clear_sentinels}
				(if (i64.ne (call $take (global.get $call) (i32.const {destination})
					(i32.const {size})) (i64.const {size})) (then unreachable))
				{sentinels}
				{exhaustive_check}
				(call $send))
		)"#
	)
}

fn host(size: usize, exhaustive: bool) -> WasmHost<HeadlessBackend> {
	let mut host = WasmHost::headless(guest(size, exhaustive).as_bytes()).expect("compile guest");
	let plugin = host
		.plugins()
		.register("echo", [MethodId(0)])
		.expect("register echo");
	assert_eq!(plugin.0, 0);
	host.plugins()
		.grant(plugin, MethodId(0), true)
		.expect("grant echo");

	if exhaustive {
		host.set_fuel_budget(100_000_000);
	}

	host.start(1.0, 1.0).expect("start guest");
	host
}

fn next(host: &mut WasmHost<HeadlessBackend>, size: usize) -> NativeRequest {
	let mut request = MaybeUninit::uninit();
	// SAFETY: The host exclusively lends its live bridge for this call; the
	// aligned output takes ownership of exactly one request buffer.
	let request = unsafe {
		assert_eq!(
			lk_bridge_next_request(host.plugins(), request.as_mut_ptr()),
			1
		);
		request.assume_init()
	};
	assert_eq!(request.payload.len, size);
	assert_eq!(request.plugin, 0);
	assert_eq!(request.method, 0);
	request
}

fn roundtrip(host: &mut WasmHost<HeadlessBackend>, size: usize, completion: Completion) {
	let request = next(host, size);
	// SAFETY: Access remains on the owning thread. The copied path borrows a
	// live request buffer then releases it once. The owned path transfers that
	// buffer exactly once and retains no view into it.
	unsafe {
		let result = match completion {
			Completion::Owned => {
				lk_bridge_complete_owned(host.plugins(), request.id, request.payload)
			}
			Completion::Copied => {
				let result = lk_bridge_complete(
					host.plugins(),
					request.id,
					0,
					request.payload.data,
					request.payload.len,
				);
				lk_buffer_release(request.payload);
				result
			}
		};

		assert_eq!(black_box(result), 0, "native completion failed");
	}
	host.wake_plugins()
		.expect("guest must consume, validate, and submit next call");
}

fn finish(host: &mut WasmHost<HeadlessBackend>, size: usize) {
	let request = next(host, size);
	// SAFETY: The dequeued request owns a live buffer, released exactly once.
	unsafe { lk_buffer_release(request.payload) };
	host.plugins()
		.cancel(CallId(request.id))
		.expect("cancel final queued call");
	assert_eq!(host.plugins().buffered_bytes(), 0);
	assert!(host.plugins().next_request().is_none());
}

fn verify_negative_controls() {
	for control in 0..3 {
		let mut host = host(64, true);
		let request = next(&mut host, 64);
		let mut reply = vec![42; 64];

		let error = match control {
			0 => {
				// An interior byte outside all timed sentinels must fail
				// exhaustive verification.
				reply[7] = 0;
				0
			}
			1 => {
				reply.pop();
				0
			}
			_ => Error::Native as i32,
		};

		// SAFETY: The host and reply are live exclusive borrows. Completion
		// copies reply bytes, and the original request is released once.
		unsafe {
			assert_eq!(
				lk_bridge_complete(
					host.plugins(),
					request.id,
					error,
					reply.as_ptr(),
					reply.len()
				),
				0
			);
			lk_buffer_release(request.payload);
		}
		assert!(
			host.wake_plugins().is_err(),
			"invalid response escaped guest validation"
		);
		assert_eq!(host.plugins().buffered_bytes(), 0);

		assert!(host.plugins().next_request().is_none());
	}
}

fn verify_size(size: usize, modes: [Completion; 2]) {
	for mode in modes {
		let mut checked = host(size, true);

		for _ in 0..3 {
			roundtrip(&mut checked, size, mode);
		}

		finish(&mut checked, size);
	}
}

fn warm(hosts: &mut [WasmHost<HeadlessBackend>; 2], size: usize, modes: [Completion; 2]) {
	for (host, mode) in hosts.iter_mut().zip(modes) {
		for _ in 0..100 {
			roundtrip(host, size, mode);
		}
	}
}

fn sample(
	hosts: &mut [WasmHost<HeadlessBackend>; 2],
	size: usize,
	modes: [Completion; 2],
	iterations: u32,
) -> [Vec<f64>; 2] {
	let mut samples = [Vec::new(), Vec::new()];

	for repetition in 0..5 {
		// Reverse pair order as well as alternating paths to reduce drift.
		for index in if repetition % 2 == 0 { [0, 1] } else { [1, 0] } {
			let started = Instant::now();

			for _ in 0..iterations {
				roundtrip(&mut hosts[index], size, modes[index]);
			}

			samples[index].push(started.elapsed().as_secs_f64() * 1e9 / f64::from(iterations));
		}
	}

	samples
}

fn report(
	host: &mut WasmHost<HeadlessBackend>,
	size: usize,
	mode: Completion,
	samples: &mut [f64],
	iterations: u32,
) {
	ALLOCATIONS.set(Some((0, 0)));

	for _ in 0..100 {
		roundtrip(host, size, mode);
	}

	let (allocations, bytes) = ALLOCATIONS.replace(None).expect("counting enabled");
	samples.sort_by(f64::total_cmp);
	println!(
		"{size} bytes {}: median {:.1} ns, range {:.1}..{:.1}; {} successful timed calls; 0 \
		 errors; {:.2} allocations / {:.2} allocated bytes per call",
		mode.name(),
		samples[2],
		samples[0],
		samples[4],
		5 * iterations,
		f64::from(u32::try_from(allocations).expect("allocation count")) / 100.0,
		f64::from(u32::try_from(bytes).expect("allocated byte count")) / 100.0,
	);
	finish(host, size);
}

fn measure_size(size: usize, modes: [Completion; 2]) {
	const SMALL_ITERATIONS: u32 = 200_000;
	const LARGE_ITERATIONS: u32 = 2_000;
	let mut hosts = [host(size, false), host(size, false)];
	warm(&mut hosts, size, modes);
	let iterations = if size < 1024 * 1024 {
		SMALL_ITERATIONS
	} else {
		LARGE_ITERATIONS
	};

	let mut samples = sample(&mut hosts, size, modes, iterations);

	for (index, mode) in modes.into_iter().enumerate() {
		report(
			&mut hosts[index],
			size,
			mode,
			&mut samples[index],
			iterations,
		);
	}
}

fn main() {
	const SMALL_PAYLOAD: usize = 64;
	const MEDIUM_PAYLOAD: usize = 4096;
	verify_negative_controls();
	println!("Native echo transport; 5 interleaved samples; concurrency=1; failures abort.");
	println!(
		"Negative controls passed: interior corruption, wrong length, and native error each \
		 trapped."
	);
	println!(
		"Full-byte guest verification runs separately; timed calls verify length and 3 reset \
		 sentinels."
	);
	println!(
		"Allocation counts are separate from timing; bytes are requested allocation volume, not \
		 peak memory."
	);

	for size in [0, SMALL_PAYLOAD, MEDIUM_PAYLOAD, 1024 * 1024] {
		let modes = [Completion::Owned, Completion::Copied];
		verify_size(size, modes);

		if cfg!(debug_assertions) {
			println!("{size} bytes: verification passed; run cargo bench for release timings");
			continue;
		}

		measure_size(size, modes);
	}
}
