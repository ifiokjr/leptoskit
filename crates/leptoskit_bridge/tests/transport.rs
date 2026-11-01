use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Event;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

const READ: MethodId = MethodId(0);

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
#[test]
fn revocation_generation_survives_queueing_running_and_regrant() {
	for running in [false, true] {
		let (mut bridge, plugin) = bridge(Limits::default());
		let old = bridge.submit(plugin, READ, Bytes::new()).unwrap();
		let old_permission = bridge.permission(old).unwrap();

		if running {
			assert_eq!(bridge.next_request().unwrap().id, old);
		}

		bridge.grant(plugin, READ, false).unwrap();
		bridge.grant(plugin, READ, true).unwrap();

		assert!(old_permission.is_cancelled());
		assert!(bridge.permission(old).unwrap().is_cancelled());

		if !running {
			assert_eq!(bridge.next_request().unwrap().id, old);
		}

		let new = bridge.submit(plugin, READ, Bytes::new()).unwrap();
		assert!(!bridge.permission(new).unwrap().is_cancelled());
		assert_eq!(bridge.next_request().unwrap().id, new);
		// A late successful native completion cannot revive the old grant.
		bridge
			.complete(old, Ok(Bytes::from_static(b"old")))
			.unwrap();
		bridge
			.complete(new, Ok(Bytes::from_static(b"new")))
			.unwrap();
		assert_eq!(
			bridge.take_result(old).unwrap(),
			Some(Err(Error::PermissionDenied))
		);
		assert_eq!(
			bridge.take_result(new).unwrap(),
			Some(Ok(Bytes::from_static(b"new")))
		);
		assert_eq!(bridge.buffered_bytes(), 0);
	}
}

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
#[test]
fn revoking_one_method_does_not_cancel_other_grants() {
	let mut bridge = Bridge::default();
	let plugin = bridge.register("scoped", [READ, MethodId(1)]).unwrap();

	for method in [READ, MethodId(1)] {
		bridge.grant(plugin, method, true).unwrap();
	}

	let read = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	let other = bridge.submit(plugin, MethodId(1), Bytes::new()).unwrap();
	bridge.grant(plugin, READ, false).unwrap();
	assert!(bridge.permission(read).unwrap().is_cancelled());
	assert!(!bridge.permission(other).unwrap().is_cancelled());
	bridge.grant(plugin, MethodId(1), true).unwrap();
	assert!(!bridge.permission(other).unwrap().is_cancelled());
}

#[test]
fn reused_slots_reject_stale_completions_without_blocking_other_slots() {
	let (mut bridge, plugin) = bridge(Limits {
		calls: 3,
		..Limits::default()
	});
	let first = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	let held = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.next_request().unwrap();
	bridge.cancel(first).unwrap();
	assert_eq!(bridge.next_cancelled(), Some(first));

	for _ in 0..1000 {
		let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
		assert_ne!(id, first);
		let request = bridge.next_request().unwrap();
		assert_eq!(request.id, id);
		assert_eq!(
			bridge.complete(first, Ok(Bytes::new())),
			Err(Error::InvalidCall)
		);
		bridge.complete(id, Ok(Bytes::from_static(b"ok"))).unwrap();
		assert_eq!(
			bridge.take_result(id).unwrap(),
			Some(Ok(Bytes::from_static(b"ok")))
		);
	}

	bridge
		.complete(held, Ok(Bytes::from_static(b"held")))
		.unwrap();
	assert_eq!(
		bridge.take_result(held).unwrap(),
		Some(Ok(Bytes::from_static(b"held")))
	);
}

fn bridge(limits: Limits) -> (Bridge, PluginId) {
	let mut bridge = Bridge::new(limits);
	let plugin = bridge.register("test", [READ]).unwrap();
	bridge.grant(plugin, READ, true).unwrap();
	(bridge, plugin)
}

#[test]
fn payload_ownership_moves_through_requests_and_replies() {
	let (mut bridge, plugin) = bridge(Limits::default());
	let payload = Bytes::from(vec![42; 1024 * 1024]);
	let allocation = payload.as_ptr();
	let id = bridge.submit(plugin, READ, payload).unwrap();
	assert_eq!(bridge.buffered_bytes(), 1024 * 1024);
	assert_eq!(bridge.result(id).unwrap(), None);
	let request = bridge.next_request().unwrap();
	assert_eq!(request.payload.as_ptr(), allocation);

	assert_eq!(bridge.buffered_bytes(), 0);
	bridge.complete(id, Ok(request.payload)).unwrap();
	let result = bridge.take_result(id).unwrap().unwrap().unwrap();
	assert_eq!(result.as_ptr(), allocation);
	assert_eq!(bridge.buffered_bytes(), 0);
	assert_eq!(bridge.result(id), Err(Error::InvalidCall));
}

#[test]
fn guests_cannot_invoke_uninstalled_ungranted_or_unknown_methods() {
	let mut bridge = Bridge::default();
	let plugin = bridge.register("camera", [READ]).unwrap();
	assert_eq!(bridge.resolve("camera"), Some(plugin));
	assert_eq!(bridge.register("camera", []), Err(Error::InvalidArgument));
	assert_eq!(
		bridge.submit(plugin, READ, Bytes::new()),
		Err(Error::PermissionDenied)
	);
	bridge.grant(plugin, READ, true).unwrap();
	assert!(bridge.submit(plugin, READ, Bytes::new()).is_ok());
	bridge.grant(plugin, READ, false).unwrap();

	assert_eq!(
		bridge.submit(plugin, READ, Bytes::new()),
		Err(Error::PermissionDenied)
	);
	assert_eq!(
		bridge.submit(plugin, MethodId(99), Bytes::new()),
		Err(Error::Unavailable)
	);
	assert_eq!(
		bridge.submit(PluginId(99), READ, Bytes::new()),
		Err(Error::Unavailable)
	);
}

#[test]
fn queued_running_and_ready_calls_can_be_cancelled() {
	let (mut bridge, plugin) = bridge(Limits::default());
	let queued = bridge
		.submit(plugin, READ, Bytes::from_static(b"a"))
		.unwrap();
	bridge.cancel(queued).unwrap();
	assert!(bridge.next_request().is_none());
	assert_eq!(bridge.buffered_bytes(), 0);
	let running = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge.cancel(running).unwrap();

	assert_eq!(bridge.result(running), Err(Error::InvalidCall));
	assert_eq!(
		bridge.complete(running, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);
	let ready = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	assert_ne!(ready, running);
	bridge.next_request().unwrap();
	bridge
		.complete(ready, Ok(Bytes::from_static(b"result")))
		.unwrap();
	assert_eq!(
		bridge.complete(ready, Ok(Bytes::new())),
		Err(Error::InvalidCall)
	);
	bridge.cancel(ready).unwrap();

	assert_eq!(bridge.next_cancelled(), Some(running));
	assert_eq!(bridge.next_cancelled(), None);
	assert_eq!(bridge.buffered_bytes(), 0);
}

#[test]
fn running_cancellations_remain_bounded_until_the_host_observes_them() {
	let (mut bridge, plugin) = bridge(Limits {
		calls: 1,
		..Limits::default()
	});

	for _ in 0..1000 {
		let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
		bridge.next_request().unwrap();
		bridge.cancel(id).unwrap();
		assert_eq!(bridge.cancel(id), Err(Error::InvalidCall));
		assert_eq!(bridge.submit(plugin, READ, Bytes::new()), Err(Error::Busy));
		assert_eq!(
			bridge.complete(id, Ok(Bytes::new())),
			Err(Error::InvalidCall)
		);

		assert_eq!(bridge.next_cancelled(), Some(id));
		assert_eq!(bridge.next_cancelled(), None);
		assert_eq!(bridge.result(id), Err(Error::InvalidCall));
	}
}

#[test]
fn unread_results_keep_the_call_limit_and_oversized_replies_release_cleanly() {
	let limits = Limits {
		calls: 1,
		payload_bytes: 4,
		..Limits::default()
	};
	let (mut bridge, plugin) = bridge(limits);
	assert_eq!(
		bridge.submit(plugin, READ, Bytes::from_static(b"large")),
		Err(Error::TooLarge)
	);
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	bridge
		.complete(id, Ok(Bytes::from_static(b"large")))
		.unwrap();
	assert_eq!(bridge.submit(plugin, READ, Bytes::new()), Err(Error::Busy));
	assert_eq!(bridge.take_result(id), Ok(Some(Err(Error::TooLarge))));

	assert!(bridge.submit(plugin, READ, Bytes::new()).is_ok());
}

#[test]
fn events_and_replies_share_a_byte_budget_and_overflow_is_explicit() {
	let limits = Limits {
		events: 1,
		buffered_bytes: 4,
		..Limits::default()
	};
	let (mut bridge, plugin) = bridge(limits);
	let id = bridge.submit(plugin, READ, Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	let event = || {
		Event {
			plugin,
			kind: 7,
			payload: Bytes::from_static(b"1234"),
		}
	};
	bridge.emit(event()).unwrap();
	assert_eq!(bridge.emit(event()), Err(Error::Busy));
	bridge.complete(id, Ok(Bytes::from_static(b"x"))).unwrap();

	assert_eq!(bridge.take_result(id), Ok(Some(Err(Error::Busy))));
	let received = bridge.next_event().unwrap();
	assert_eq!(received.kind, 7);
	assert_eq!(received.payload.as_ref(), b"1234");
	assert_eq!(bridge.buffered_bytes(), 0);
	bridge.emit(event()).unwrap();
}

#[test]
fn binary_decoder_borrows_and_rejects_invalid_inputs() {
	let mut encoder = Encoder::with_capacity(12);
	encoder.u32(7);
	encoder.bytes(b"hello").unwrap();
	let bytes = encoder.finish();
	let mut decoder = Decoder::new(&bytes);
	assert_eq!(decoder.u32(), Ok(7));
	let value = decoder.string().unwrap();
	assert_eq!(value, "hello");

	assert_eq!(value.as_ptr(), bytes[8..].as_ptr());
	decoder.finish().unwrap();

	for length in 0..bytes.len() {
		let mut decoder = Decoder::new(&bytes[..length]);
		let result = decoder.u32().and_then(|_| decoder.string());
		assert!(result.is_err());
	}

	assert_eq!(
		Decoder::new(b"trailing").finish(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Decoder::new(&[1, 0, 0, 0, 255]).string(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Decoder::new(&[255, 255, 255, 255]).bytes(),
		Err(Error::InvalidArgument)
	);
}
