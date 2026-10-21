//! Compiled guest regression for malformed plugin acknowledgements and batch
//! replies.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::Cell;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_plugin_filesystem::guest::Filesystem;
use leptoskit_plugin_haptics::Impact;
use leptoskit_plugin_haptics::Notification;
use leptoskit_plugin_haptics::guest::Haptics;
use leptoskit_plugin_sqlite::Statement;
use leptoskit_plugin_sqlite::guest::Sqlite;

type Task = Pin<Box<dyn Future<Output = ()>>>;

thread_local! {
	static TASK: RefCell<Option<Task>> = RefCell::new(None);
	static DONE: Cell<bool> = const { Cell::new(false) };
}

/// Starts the probe, first rejecting malformed replies, then accepting valid
/// ones.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	TASK.with_borrow_mut(|task| {
		*task = Some(Box::pin(async {
			let filesystem = Filesystem::new().unwrap();
			let haptics = Haptics::new().unwrap();
			let sqlite = Sqlite::new().unwrap();
			for valid in [false, true] {
				let expected = if valid {
					Ok(())
				} else {
					Err(Error::InvalidArgument)
				};
				assert_eq!(filesystem.write("test", b"content").await, expected);
				assert_eq!(filesystem.mkdir("directory").await, expected);
				assert_eq!(filesystem.remove("test").await, expected);
				assert_eq!(filesystem.rename("source", "destination").await, expected);
				assert_eq!(haptics.selection().await, expected);
				assert_eq!(haptics.impact(Impact::Light).await, expected);
				assert_eq!(haptics.notification(Notification::Success).await, expected);
				assert_eq!(haptics.android_feedback(0).await, expected);
				let statements = [
					Statement::new("INSERT INTO t VALUES (1)", vec![]),
					Statement::new("INSERT INTO t VALUES (2)", vec![]),
				];
				let result = sqlite.transaction(&statements).await;
				if valid {
					assert_eq!(result.unwrap().len(), 2);
				} else {
					assert_eq!(result, Err(Error::InvalidArgument));
				}
			}
			DONE.with(|done| done.set(true));
		}));
	});
	leptoskit_poll();
}

/// Advances the probe after the host delivers each plugin reply.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	TASK.with_borrow_mut(|task| {
		if task.as_mut().is_some_and(|future| {
			future
				.as_mut()
				.poll(&mut Context::from_waker(Waker::noop()))
				.is_ready()
		}) {
			*task = None;
		}
	});
}

/// Reports completion so the host verifies every assertion was reached.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(_id: i64, _kind: i32, _ptr: i32, _len: i32) -> i32 {
	DONE.with(|done| i32::from(done.get()))
}
