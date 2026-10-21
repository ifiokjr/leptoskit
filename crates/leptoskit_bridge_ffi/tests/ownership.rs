#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr;
use std::slice;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_bridge_ffi::*;

#[test]
fn returned_buffers_outlive_their_bridge() {
	// SAFETY: All pointers here are borrowed from live local storage or are
	// ownership handles returned by the FFI; every owner is released once.
	unsafe {
		let bridge = lk_bridge_create();
		assert!(!bridge.is_null());
		assert_eq!(
			lk_bridge_register(bridge, b"echo".as_ptr(), 4, [0].as_ptr(), 1),
			0
		);
		assert_eq!(lk_bridge_grant(bridge, 0, 0, true), 0);
		let id = lk_bridge_submit(bridge, 0, 0, b"hello".as_ptr(), 5);
		assert!(id > 0);
		let mut request = MaybeUninit::uninit();
		assert_eq!(lk_bridge_next_request(bridge, request.as_mut_ptr()), 1);
		let request = request.assume_init();
		lk_bridge_destroy(bridge);
		assert_eq!(
			slice::from_raw_parts(request.payload.data, request.payload.len),
			b"hello"
		);
		lk_buffer_release(request.payload);
	}
}

#[test]
fn rejected_foreign_payloads_are_checked_before_copying() {
	let mut bridge = Bridge::new(Limits {
		payload_bytes: 4,
		..Limits::default()
	});
	let plugin = bridge.register("echo", [MethodId(0)]).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();
	let id = bridge.submit(plugin, MethodId(0), Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	// SAFETY: The borrowed bridge is valid; oversized buffers are explicitly
	// rejected before access, and null with zero length is a supported ABI
	// input.
	unsafe {
		assert_eq!(
			lk_bridge_submit(&raw mut bridge, plugin.0, 0, ptr::null(), 5),
			-5
		);
		assert_eq!(
			lk_bridge_complete(&raw mut bridge, id.0, 0, ptr::null(), 5),
			0
		);
	}
	assert_eq!(bridge.take_result(id), Ok(Some(Err(Error::TooLarge))));
	assert_eq!(bridge.buffered_bytes(), 0);
}

#[test]
fn null_handles_and_output_pointers_fail_without_consuming_work() {
	let mut bridge = Bridge::default();
	let plugin = bridge.register("echo", [MethodId(0)]).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();
	bridge.submit(plugin, MethodId(0), Bytes::new()).unwrap();
	// SAFETY: Null pointers are explicitly supported as invalid arguments.
	unsafe {
		assert_eq!(lk_bridge_grant(ptr::null_mut(), 0, 0, true), -3);
		assert_eq!(lk_bridge_next_request(&raw mut bridge, ptr::null_mut()), -3);
	}
	assert!(bridge.next_request().is_some());
}

#[test]
fn owned_completion_preserves_the_payload_allocation_and_rejects_late_work() {
	let mut bridge = Bridge::default();
	let plugin = bridge.register("echo", [MethodId(0)]).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();
	for cancelled in [false, true] {
		let payload = Bytes::from(vec![42; 4096]);
		let pointer = payload.as_ptr();
		let id = bridge.submit(plugin, MethodId(0), payload).unwrap();
		let mut request = MaybeUninit::uninit();
		// SAFETY: Local aligned output storage takes ownership of one request.
		let request = unsafe {
			assert_eq!(
				lk_bridge_next_request(&raw mut bridge, request.as_mut_ptr()),
				1
			);
			request.assume_init()
		};
		assert_eq!(request.payload.data, pointer);
		if cancelled {
			bridge.cancel(id).unwrap();
		}
		// SAFETY: Transfer the request buffer once, retaining no borrowed view.
		let status = unsafe { lk_bridge_complete_owned(&raw mut bridge, id.0, request.payload) };
		if cancelled {
			assert_eq!(status, -(Error::InvalidCall as i32));
		} else {
			assert_eq!(status, 0);
			let reply = bridge.take_result(id).unwrap().unwrap().unwrap();
			assert_eq!(reply.as_ptr(), pointer);
			assert_eq!(reply.as_ref(), &[42; 4096]);
		}
	}
}
