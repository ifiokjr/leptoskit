//! Compiled guest fixture exercising real async calls, event wakeups, and
//! drops.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Wake;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;
use leptoskit_bridge::guest::Events;

type Task = Pin<Box<dyn Future<Output = ()>>>;

thread_local! {
	static TASK: RefCell<Option<Task>> = RefCell::new(None);
}
static WOKEN: AtomicBool = AtomicBool::new(true);

struct Ready;

impl Wake for Ready {
	fn wake(self: Arc<Self>) {
		WOKEN.store(true, Ordering::Relaxed);
	}
}

fn poll() {
	assert!(
		WOKEN.swap(false, Ordering::Relaxed),
		"host did not wake the guest"
	);
	let waker = Waker::from(Arc::new(Ready));
	TASK.with_borrow_mut(|task| {
		if task
			.as_mut()
			.unwrap()
			.as_mut()
			.poll(&mut Context::from_waker(&waker))
			.is_ready()
		{
			*task = None;
		}
	});
}

async fn calls(client: Client) {
	assert!(matches!(
		client.call(MethodId(1), &[]),
		Err(Error::PermissionDenied)
	));
	// Dropping an unpolled call must remove its native request.
	drop(client.call(MethodId(0), b"cancel me").unwrap());
	let running = client.call(MethodId(0), b"cancel running").unwrap();
	let mut echo = client.call(MethodId(0), b"echo").unwrap();
	let echo_id = echo.id().unwrap();
	let mut identities = client.plugin_id().0.to_le_bytes().to_vec();
	identities.extend_from_slice(&running.id().unwrap().0.to_le_bytes());
	identities.extend_from_slice(&echo_id.0.to_le_bytes());
	let mut routed = client.call(MethodId(0), &identities).unwrap();
	assert!(routed.id().is_some());
	assert_eq!((&mut routed).await, Err(Error::Cancelled));
	assert_eq!(routed.id(), None);
	assert_eq!(echo.id(), Some(echo_id));
	let reply = (&mut echo).await.unwrap();
	assert_eq!(echo.id(), None);
	assert_eq!(reply.as_ref(), b"echo");
	drop(running);
}

/// Starts the asynchronous probe and suspends on its first native reply.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	TASK.with_borrow_mut(|task| {
		*task = Some(Box::pin(async {
			let client = Client::new("echo").unwrap();
			calls(client).await;
			let mut events = Events::take().unwrap();
			assert!(matches!(Events::take(), Err(Error::Busy)));
			// Cancel a pending wait; a subsequent wait must still receive its
			// event.
			std::future::poll_fn(|context| {
				let mut wait = events.recv();
				assert!(Pin::new(&mut wait).poll(context).is_pending());
				std::task::Poll::Ready(())
			})
			.await;
			let event = events.recv().await.unwrap();
			assert_eq!(event.plugin, client.plugin_id());
			assert_eq!(event.kind, 42);
			assert_eq!(event.payload.as_ref(), b"native event");
			drop(events);
			let mut events = Events::take().unwrap();
			assert!(events.try_recv().unwrap().is_none());
			client.call(MethodId(2), b"passed").unwrap().await.unwrap();
		}));
	});
	poll();
}

/// The probe's tiny executor is driven by the host after plugin wakeups.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(_id: i64, _kind: i32, _ptr: i32, _len: i32) -> i32 {
	poll();
	1
}
