use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use wasm_split_helpers::SplitLoaderError;

thread_local! {
	static PENDING: RefCell<Option<Pin<Box<dyn Future<Output = Result<String, SplitLoaderError>>>>>> = RefCell::new(None);
}

#[wasm_split_helpers::wasm_split(details, fallible)]
fn details(input: String) -> Result<String, SplitLoaderError> {
	Ok(format!("loaded: {input}"))
}

#[unsafe(no_mangle)]
pub extern "C" fn begin() {
	PENDING.with_borrow_mut(|pending| {
		*pending = Some(Box::pin(details(String::from("shared input"))));
	});
}

#[unsafe(no_mangle)]
pub extern "C" fn poll() -> i32 {
	PENDING.with_borrow_mut(|pending| {
		let future = pending.as_mut().expect("begin before poll");
		let mut context = Context::from_waker(Waker::noop());

		match future.as_mut().poll(&mut context) {
			Poll::Pending => 0,
			Poll::Ready(Ok(value)) => {
				assert_eq!(value, "loaded: shared input");
				*pending = None;
				1
			}
			Poll::Ready(Err(_)) => {
				*pending = None;
				2
			}
		}
	})
}
