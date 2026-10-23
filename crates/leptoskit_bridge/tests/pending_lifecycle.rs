//! Pending calls preserve unrelated queued work and surface native failures.

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;

const READ: MethodId = MethodId(0);

#[test]
fn cancelling_one_queued_call_preserves_other_requests_and_pending_results() {
	let (mut bridge, plugin) = bridge(Limits::default());
	let cancelled = bridge
		.submit(plugin, READ, Bytes::from_static(b"cancel"))
		.unwrap();
	let retained = bridge
		.submit(plugin, READ, Bytes::from_static(b"keep"))
		.unwrap();

	assert_eq!(bridge.take_result(retained), Ok(None));
	bridge.cancel(cancelled).unwrap();
	let request = bridge.next_request().unwrap();
	assert_eq!(request.id, retained);
	assert_eq!(request.payload.as_ref(), b"keep");
	assert!(bridge.next_request().is_none());
	assert_eq!(bridge.take_result(retained), Ok(None));
	assert_eq!(bridge.take_result(cancelled), Err(Error::InvalidCall));
	bridge.complete(retained, Err(Error::Native)).unwrap();

	assert_eq!(bridge.result(retained), Ok(Some(&Err(Error::Native))));
	assert_eq!(bridge.take_result(retained), Ok(Some(Err(Error::Native))));
	assert_eq!(bridge.take_result(retained), Err(Error::InvalidCall));
	assert_eq!(bridge.buffered_bytes(), 0);
	assert!(bridge.next_cancelled().is_none());
}

fn bridge(limits: Limits) -> (Bridge, PluginId) {
	let mut bridge = Bridge::new(limits);
	let plugin = bridge.register("test", [READ]).unwrap();
	bridge.grant(plugin, READ, true).unwrap();
	(bridge, plugin)
}
