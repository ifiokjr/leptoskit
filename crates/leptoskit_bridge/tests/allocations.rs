//! Verify the hot-path allocation contract independently of timing noise.
#![allow(unsafe_code)]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;

struct CountingAllocator;

thread_local! {
	static COUNTING: Cell<bool> = const { Cell::new(false) };
	static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
	if COUNTING.try_with(Cell::get).unwrap_or(false) {
		let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
	}
}

// SAFETY: All operations delegate to System with unchanged arguments.
// The extra bookkeeping uses non-allocating thread-local Cells.
unsafe impl GlobalAlloc for CountingAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		count();
		// SAFETY: Forward the caller's allocation contract unchanged.
		unsafe { System.alloc(layout) }
	}

	unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
		// SAFETY: Forward the original pointer and layout to the same
		// allocator.
		unsafe { System.dealloc(pointer, layout) }
	}

	unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
		count();
		// SAFETY: Forward the caller's reallocation contract unchanged.
		unsafe { System.realloc(pointer, layout, size) }
	}
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn initialized_rust_transport_does_not_allocate_per_call() {
	let mut bridge = Bridge::default();
	let plugin = bridge.register("echo", [MethodId(0)]).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();
	let mut payload = Bytes::from(vec![42; 8192]);
	let pointer = payload.as_ptr();
	ALLOCATIONS.set(0);
	COUNTING.set(true);
	for _ in 0..1000 {
		let id = bridge.submit(plugin, MethodId(0), payload).unwrap();
		let request = bridge.next_request().unwrap();
		bridge.complete(id, Ok(request.payload)).unwrap();
		payload = bridge.take_result(id).unwrap().unwrap().unwrap();
		assert_eq!(payload.as_ptr(), pointer);
		assert_eq!(payload.len(), 8192);
	}
	COUNTING.set(false);
	assert_eq!(
		ALLOCATIONS.get(),
		0,
		"transport allocated after initialization"
	);
}
