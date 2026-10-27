use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use futures::channel::oneshot;
use futures::executor::block_on;
use futures::task::noop_waker;
use leptoskit_use::AsyncState;
use leptoskit_use::use_async_state;
use parking_lot::Mutex;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;

/// One fetch implementation shared by every scenario so the hook has a single
/// generic instantiation in this binary: each call takes the next scripted
/// receiver from the queue; an empty queue pends forever.
type Scripted = Arc<Mutex<Vec<oneshot::Receiver<Result<u32, String>>>>>;

fn scripted_fetch(
	queue: Scripted,
) -> impl Fn() -> Pin<Box<dyn Future<Output = Result<u32, String>> + 'static>> {
	move || {
		let next = {
			let mut scripted = queue.lock();
			if scripted.is_empty() {
				None
			} else {
				Some(scripted.remove(0))
			}
		};
		Box::pin(async move {
			match next {
				Some(receiver) => receiver.await.unwrap_or_else(|_| Err("lost".to_owned())),
				None => std::future::pending().await,
			}
		})
	}
}

#[test]
fn use_async_state_covers_success_error_drop_and_disposal() {
	let owner = Owner::new();
	let (ok_sender, ok_receiver) = oneshot::channel();
	let (err_sender, err_receiver) = oneshot::channel();
	let queue: Scripted = Arc::new(Mutex::new(vec![ok_receiver, err_receiver]));
	let fetch = scripted_fetch(Arc::clone(&queue));
	let hook = block_on(owner.with(|| use_async_state(0_u32, fetch)));

	// Success: pending during the await, then the fetched value.
	assert_eq!(hook.state.get(), AsyncState::Success(0));
	let success = (hook.execute)();
	ok_sender.send(Ok(7)).unwrap();
	block_on(success);
	assert_eq!(hook.state.get(), AsyncState::Success(7));

	// Error: the failure replaces the value and the signal stays usable.
	let error = (hook.execute)();
	err_sender.send(Err("operation failed".to_owned())).unwrap();
	block_on(error);
	assert_eq!(
		hook.state.get(),
		AsyncState::Error("operation failed".to_owned())
	);

	// Drop: the execution future is dropped mid-await, leaving Pending.
	let mut dropped = (hook.execute)();
	let waker = noop_waker();
	let mut context = Context::from_waker(&waker);
	assert_eq!(dropped.as_mut().poll(&mut context), Poll::Pending);
	assert_eq!(hook.state.get_untracked(), AsyncState::Pending);
	drop(dropped);
	assert_eq!(hook.state.get_untracked(), AsyncState::Pending);

	// Disposal mid-flight: the in-flight fetch resolves, the update is skipped.
	let mut mid_flight = (hook.execute)();
	assert_eq!(mid_flight.as_mut().poll(&mut context), Poll::Pending);
	owner.cleanup();

	// After disposal: a fresh execution returns before running its fetch.
	block_on((hook.execute)());
}

#[test]
fn state_variants_clone_and_format() {
	let pending = AsyncState::<u8, u8>::Pending;
	let success = AsyncState::<u8, u8>::Success(7);
	let error = AsyncState::<u8, u8>::Error(0);
	assert_eq!(pending.clone(), pending);
	assert_eq!(success.clone(), success);
	assert_eq!(error.clone(), error);
	assert_eq!(format!("{pending:?}"), "Pending");
	assert_eq!(format!("{success:?}"), "Success(7)");
	assert_eq!(format!("{error:?}"), "Error(0)");
	let success = AsyncState::<String, String>::Success("value".to_owned());
	let error = AsyncState::<String, String>::Error("boom".to_owned());
	assert_eq!(success.clone(), success);
	assert_eq!(error.clone(), error);
	assert_eq!(format!("{success:?}"), r#"Success("value")"#);
	assert_eq!(format!("{error:?}"), r#"Error("boom")"#);
}
