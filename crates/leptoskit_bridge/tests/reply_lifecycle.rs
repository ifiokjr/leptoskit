use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Limits;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::ReplyGuard;

struct Lease {
	discarded: Arc<AtomicUsize>,
	committed: bool,
}
impl ReplyGuard for Lease {
	fn commit(&mut self) {
		self.committed = true;
	}
}
impl Drop for Lease {
	fn drop(&mut self) {
		if !self.committed {
			self.discarded.fetch_add(1, Ordering::SeqCst);
		}
	}
}
fn reply(discarded: &Arc<AtomicUsize>) -> Reply {
	Reply::guarded(
		Bytes::from_static(b"handle"),
		Lease {
			discarded: Arc::clone(discarded),
			committed: false,
		},
	)
}
fn pending(bridge: &mut Bridge) -> leptoskit_bridge::CallId {
	let plugin = bridge
		.resolve("resource")
		.unwrap_or_else(|| bridge.register("resource", [MethodId(0)]).unwrap());
	bridge.grant(plugin, MethodId(0), true).unwrap();
	let id = bridge.submit(plugin, MethodId(0), Bytes::new()).unwrap();
	bridge.next_request().unwrap();
	id
}
#[test]
fn lease_commits_only_on_consumption_not_inspection() {
	let discarded = Arc::new(AtomicUsize::new(0));
	let mut bridge = Bridge::default();
	let id = pending(&mut bridge);
	bridge.complete_reply(id, Ok(reply(&discarded))).unwrap();
	assert_eq!(
		bridge.result(id).unwrap().unwrap().as_ref().unwrap(),
		b"handle".as_slice()
	);
	assert_eq!(discarded.load(Ordering::SeqCst), 0);
	bridge.take_result(id).unwrap().unwrap().unwrap();
	drop(bridge);

	assert_eq!(discarded.load(Ordering::SeqCst), 0);
}
#[test]
fn ready_cancel_rejection_overflow_and_app_drop_release_resource_once() {
	let discarded = Arc::new(AtomicUsize::new(0));
	let mut bridge = Bridge::default();
	let id = pending(&mut bridge);
	bridge.complete_reply(id, Ok(reply(&discarded))).unwrap();
	bridge.cancel(id).unwrap();
	assert_eq!(discarded.load(Ordering::SeqCst), 1);
	assert_eq!(
		bridge.complete_reply(id, Ok(reply(&discarded))),
		Err(Error::InvalidCall)
	);
	assert_eq!(discarded.load(Ordering::SeqCst), 2);

	let id = pending(&mut bridge);
	bridge.complete_reply(id, Ok(reply(&discarded))).unwrap();
	drop(bridge);
	assert_eq!(discarded.load(Ordering::SeqCst), 3);
	let mut bridge = Bridge::new(Limits {
		payload_bytes: 1,
		..Limits::default()
	});
	let id = pending(&mut bridge);
	bridge.complete_reply(id, Ok(reply(&discarded))).unwrap();
	assert_eq!(bridge.take_result(id).unwrap(), Some(Err(Error::TooLarge)));
	assert_eq!(discarded.load(Ordering::SeqCst), 4);
}
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
#[test]
fn revoke_discards_ready_resource_and_regrant_does_not_revive_old_call() {
	let discarded = Arc::new(AtomicUsize::new(0));
	let mut bridge = Bridge::default();
	let id = pending(&mut bridge);
	let plugin = bridge.resolve("resource").unwrap();
	let old = bridge.permission(id).unwrap();
	bridge.complete_reply(id, Ok(reply(&discarded))).unwrap();
	bridge.grant(plugin, MethodId(0), false).unwrap();
	bridge.grant(plugin, MethodId(0), true).unwrap();

	assert!(old.is_cancelled());
	assert_eq!(discarded.load(Ordering::SeqCst), 1);
	assert_eq!(
		bridge.take_result(id).unwrap(),
		Some(Err(Error::PermissionDenied))
	);
	let next = pending(&mut bridge);
	assert!(!bridge.permission(next).unwrap().is_cancelled());
}
